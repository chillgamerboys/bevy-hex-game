//! Serializable combat state and its single command reducer.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use hex_core::{
    Faction, GameCommand, IssuedCommand, LatticeCoord, PendingDecision, PlayerSeat, TilePos, Turn,
    UnitId, UnitOccupancy,
};
use hex_lattice::{
    apply_disables, channel, resolve_incoming, CellKind, LatticeSpec, LatticeState, LatticeStats,
};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::{CombatData, CombatEvent, CommandRefusal, EncounterOutcome, UnitData};

/// Frozen, content-independent policy values used by combat.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RulesProfile {
    /// Stable profile identity included in snapshots and reports.
    pub name: String,
    /// Exact movement edges granted at the start of every turn.
    pub movement_per_turn: u32,
    /// Raw lattice disables opened by the placeholder melee strike.
    pub strike_disable_count: u16,
}

impl RulesProfile {
    /// Creates one validated rules profile.
    pub fn new(name: impl Into<String>, movement_per_turn: u32) -> Result<Self, String> {
        if movement_per_turn == 0 {
            return Err("movement_per_turn must be greater than zero".to_owned());
        }
        let name = name.into();
        if name.trim().is_empty() {
            return Err("rules profile name must not be empty".to_owned());
        }
        Ok(Self {
            name,
            movement_per_turn,
            strike_disable_count: 1,
        })
    }

    /// Overrides the provisional strike count without changing command semantics.
    #[must_use]
    pub fn with_strike_disable_count(mut self, count: u16) -> Self {
        self.strike_disable_count = count;
        self
    }
}

/// Stable element names resolved before simulation starts.
///
/// IDs remain session-local; reports use these frozen names and never reach into the
/// live asset server while reducing a command.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ElementNames {
    by_id: BTreeMap<hex_core::ElementId, String>,
}

impl ElementNames {
    /// Creates an explicit id-to-name table.
    #[must_use]
    pub fn new(by_id: BTreeMap<hex_core::ElementId, String>) -> Self {
        Self { by_id }
    }

    fn name(&self, id: hex_core::ElementId) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }
}

/// Frozen arena facts published to combat.
///
/// `links` are explicit directed traversal edges. The authority never guesses
/// connectivity from a hex coordinate and therefore cannot collapse stacked surfaces
/// or reconstruct map-generator policy. Observation is similarly supplied per faction.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ArenaSnapshot {
    surfaces: BTreeSet<TilePos>,
    links: BTreeSet<(TilePos, TilePos)>,
    observed: BTreeMap<Faction, BTreeSet<TilePos>>,
}

impl ArenaSnapshot {
    /// Builds an arena from exact surfaces and directed traversal edges.
    pub fn new(
        surfaces: impl IntoIterator<Item = TilePos>,
        links: impl IntoIterator<Item = (TilePos, TilePos)>,
    ) -> Result<Self, String> {
        let surfaces: BTreeSet<_> = surfaces.into_iter().collect();
        if surfaces.is_empty() {
            return Err("combat arena must contain at least one exact surface".to_owned());
        }
        let links: BTreeSet<_> = links.into_iter().collect();
        for (from, to) in &links {
            if from == to {
                return Err(format!("arena link repeats one surface: {from:?}"));
            }
            if !surfaces.contains(from) || !surfaces.contains(to) {
                return Err(format!(
                    "arena link names a surface outside the frozen arena: {from:?} -> {to:?}"
                ));
            }
        }
        Ok(Self {
            surfaces,
            links,
            observed: BTreeMap::new(),
        })
    }

    /// Publishes the exact currently observed surfaces for one faction.
    #[must_use]
    pub fn with_observation(
        mut self,
        faction: Faction,
        surfaces: impl IntoIterator<Item = TilePos>,
    ) -> Self {
        self.observed
            .insert(faction, surfaces.into_iter().collect());
        self
    }

    /// Whether a faction's frozen input currently observes a surface.
    #[must_use]
    pub fn observes(&self, faction: Faction, position: TilePos) -> bool {
        self.observed
            .get(&faction)
            .is_some_and(|surfaces| surfaces.contains(&position))
    }

    fn validates_path(&self, path: &[TilePos]) -> bool {
        !path.is_empty()
            && path.iter().all(|position| self.surfaces.contains(position))
            && path
                .windows(2)
                .all(|edge| matches!(edge, [from, to] if self.links.contains(&(*from, *to))))
    }
}

/// Lattice facts carried by one combatant.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatLattice {
    /// Fixed inscription.
    pub spec: LatticeSpec,
    /// Mutable battle state.
    pub state: LatticeState,
    /// Per-element mana policy.
    pub stats: LatticeStats,
}

/// One serializable combatant record.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatUnit {
    /// Stable session identity.
    pub id: UnitId,
    /// Seat authorized to issue this unit's commands.
    pub seat: PlayerSeat,
    /// Side used for hostility and observation.
    pub faction: Faction,
    /// Exact occupied surface.
    pub position: TilePos,
    /// Deterministic initiative value.
    pub initiative: u32,
    /// Active turn budget, present only for the current actor.
    pub turn: Option<Turn>,
    /// Domain-level command gate. Presentation may mirror this, never derive it.
    pub busy: bool,
    /// Whether the unit has left initiative.
    pub downed: bool,
    /// Optional lattice. Harnesses may omit it for movement-only combatants.
    pub lattice: Option<CombatLattice>,
}

impl CombatUnit {
    /// Builds a live unit with no active turn and no lattice.
    #[must_use]
    pub fn new(
        id: UnitId,
        seat: PlayerSeat,
        faction: Faction,
        position: TilePos,
        initiative: u32,
    ) -> Self {
        Self {
            id,
            seat,
            faction,
            position,
            initiative,
            turn: None,
            busy: false,
            downed: false,
            lattice: None,
        }
    }

    /// Attaches exact lattice facts.
    #[must_use]
    pub fn with_lattice(
        mut self,
        spec: LatticeSpec,
        state: LatticeState,
        stats: LatticeStats,
    ) -> Self {
        self.lattice = Some(CombatLattice { spec, state, stats });
        self
    }
}

/// Aggregate facts derived at the same reducer boundary as state mutation.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct CombatMetrics {
    /// Successfully applied commands.
    pub successful_commands: u32,
    /// Refused commands.
    pub refused_commands: u32,
    /// Authoritative turns completed.
    pub turns: u32,
    /// Explicit no-action yields.
    pub idle_turns: u32,
    /// Successful Channel actions.
    pub channels: u32,
    /// Mana restored under stable element names.
    pub channelled_mana: BTreeMap<String, u32>,
    /// Consecutive completed turns with no movement or action.
    pub no_progress_current: u32,
    /// Longest no-progress stretch.
    pub no_progress_max: u32,
    progress_units: BTreeSet<UnitId>,
}

impl CombatMetrics {
    fn record_success(&mut self, command: &GameCommand) {
        self.successful_commands = self.successful_commands.saturating_add(1);
        match command {
            GameCommand::EndTurn { .. } => {
                self.idle_turns = self.idle_turns.saturating_add(1);
            }
            GameCommand::Channel { unit } => {
                self.channels = self.channels.saturating_add(1);
                self.progress_units.insert(*unit);
            }
            _ => {
                self.progress_units.insert(command.unit());
            }
        }
    }

    fn record_refusal(&mut self) {
        self.refused_commands = self.refused_commands.saturating_add(1);
    }

    fn record_turn(&mut self, unit: UnitId) {
        self.turns = self.turns.saturating_add(1);
        if self.progress_units.remove(&unit) {
            self.no_progress_current = 0;
        } else {
            self.no_progress_current = self.no_progress_current.saturating_add(1);
            self.no_progress_max = self.no_progress_max.max(self.no_progress_current);
        }
    }
}

/// The one authoritative combat state.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatState {
    /// Frozen rules profile.
    pub rules: RulesProfile,
    /// Frozen world-owned spatial and observation facts.
    pub arena: ArenaSnapshot,
    /// Frozen stable content names needed by structured outcomes.
    pub elements: ElementNames,
    /// Unit records in stable-id order.
    pub units: BTreeMap<UnitId, CombatUnit>,
    /// Initiative order in stable identities.
    pub order: Vec<UnitId>,
    /// Current order index.
    current: usize,
    /// Zero-based completed round count.
    pub round: u32,
    /// Canonical aggregate summary.
    pub metrics: CombatMetrics,
    /// One defender choice suspending further simulation commands.
    pub pending: PendingDecision,
    /// Exact accepted command transcript.
    pub commands: Vec<IssuedCommand>,
    /// Exact structured outcome transcript, including refusals.
    pub events: Vec<CombatEvent>,
    /// Terminal retained-world result.
    pub outcome: Option<EncounterOutcome>,
}

impl CombatState {
    /// Starts combat from validated frozen inputs.
    pub fn start(
        rules: RulesProfile,
        arena: ArenaSnapshot,
        elements: ElementNames,
        units: impl IntoIterator<Item = CombatUnit>,
    ) -> Result<Self, String> {
        for observed in arena.observed.values().flatten() {
            if !arena.surfaces.contains(observed) {
                return Err(format!(
                    "observation names a surface outside the frozen arena: {observed:?}"
                ));
            }
        }
        let mut by_id = BTreeMap::new();
        let mut positions = BTreeSet::new();
        for mut unit in units {
            if !arena.surfaces.contains(&unit.position) {
                return Err(format!(
                    "unit {:?} stands outside the frozen arena at {:?}",
                    unit.id, unit.position
                ));
            }
            if !positions.insert(unit.position) {
                return Err(format!(
                    "two units occupy the same exact opening surface {:?}",
                    unit.position
                ));
            }
            unit.turn = None;
            let id = unit.id;
            if by_id.insert(id, unit).is_some() {
                return Err(format!("duplicate stable unit id {id:?}"));
            }
        }
        if by_id.is_empty() {
            return Err("combat roster must not be empty".to_owned());
        }
        let mut order: Vec<_> = by_id
            .values()
            .filter(|unit| !unit.downed)
            .map(|unit| unit.id)
            .collect();
        order.sort_by_key(|id| {
            let initiative = by_id.get(id).map_or(0, |unit| unit.initiative);
            (Reverse(initiative), *id)
        });
        let mut state = Self {
            rules,
            arena,
            elements,
            units: by_id,
            order,
            current: 0,
            round: 0,
            metrics: CombatMetrics::default(),
            pending: PendingDecision::None,
            commands: Vec::new(),
            events: Vec::new(),
            outcome: None,
        };
        state.grant_current_turn();
        Ok(state)
    }

    /// Stable identity currently allowed to act.
    #[must_use]
    pub fn current(&self) -> Option<UnitId> {
        self.order.get(self.current).copied()
    }

    /// Applies one intent through the sole validation and mutation boundary.
    ///
    /// A refusal appends a typed event and otherwise leaves domain state unchanged.
    pub fn apply(&mut self, issued: IssuedCommand) -> Result<(), CommandRefusal> {
        let checkpoint = self.clone();
        match self.apply_inner(&issued) {
            Ok(()) => {
                self.metrics.record_success(&issued.command);
                self.commands.push(issued);
                self.advance_if_finished();
                Ok(())
            }
            Err(refusal) => {
                *self = checkpoint;
                self.metrics.record_refusal();
                self.events.push(CombatEvent::CommandRefused {
                    command: issued.command,
                    refusal: refusal.clone(),
                });
                Err(refusal)
            }
        }
    }

    fn apply_inner(&mut self, issued: &IssuedCommand) -> Result<(), CommandRefusal> {
        if let Some(outcome) = self.outcome {
            return Err(CommandRefusal::EncounterResolved { outcome });
        }
        let unit = issued.command.unit();
        let actor = self.units.get(&unit).ok_or(CommandRefusal::UnknownUnit)?;
        if actor.seat != issued.seat {
            return Err(CommandRefusal::WrongSeat {
                issued_by: issued.seat,
                owned_by: actor.seat,
            });
        }
        if let PendingDecision::ChooseDisables { decider, .. } = self.pending {
            if !matches!(
                issued.command,
                GameCommand::ChooseDisables {
                    unit: answerer,
                    ..
                } if answerer == decider
            ) {
                return Err(CommandRefusal::DecisionPending { decider });
            }
        }
        match &issued.command {
            GameCommand::MoveAlong { unit, path } => self.apply_move(*unit, path),
            GameCommand::Strike { unit, target } => self.apply_strike(*unit, *target),
            GameCommand::EndTurn { unit } => self.apply_end_turn(*unit),
            GameCommand::Channel { unit } => self.apply_channel(*unit),
            GameCommand::ChooseDisables { unit, cells } => self.apply_choose_disables(*unit, cells),
            // These commands require resolved spell/effect or exploration-party
            // adapters that are deliberately not reconstructed from authored/live
            // resources inside the pure authority.
            GameCommand::MoveParty { .. } => Err(CommandRefusal::PartyMovementUnavailable),
            GameCommand::ChooseRestores { .. } => Err(CommandRefusal::RestorationUnavailable),
            GameCommand::Rest { .. } => Err(CommandRefusal::RestUnavailable),
            GameCommand::Cast { .. } => Err(CommandRefusal::MissingCombatData {
                data: CombatData::ContentTables,
            }),
        }
    }

    fn validate_actor(&self, unit: UnitId) -> Result<&CombatUnit, CommandRefusal> {
        let actor = self.units.get(&unit).ok_or(CommandRefusal::UnknownUnit)?;
        if actor.downed {
            return Err(CommandRefusal::ActingUnitDowned { unit });
        }
        if self.current() != Some(unit) {
            return Err(CommandRefusal::NotCurrentTurn {
                current: self.current(),
            });
        }
        if actor.busy {
            return Err(CommandRefusal::Busy);
        }
        if actor.turn.is_none() {
            return Err(CommandRefusal::NoTurn);
        }
        Ok(actor)
    }

    fn apply_move(&mut self, unit: UnitId, path: &[TilePos]) -> Result<(), CommandRefusal> {
        let actor = self.validate_actor(unit)?;
        if path.first().copied() != Some(actor.position) || !self.arena.validates_path(path) {
            return Err(CommandRefusal::InvalidPath);
        }
        let cost = u32::try_from(path.len().saturating_sub(1)).unwrap_or(u32::MAX);
        let remaining = actor.turn.map_or(0, |turn| turn.movement_left);
        if cost > remaining {
            return Err(CommandRefusal::MovementBudgetExceeded { cost, remaining });
        }
        let occupancy = UnitOccupancy::from_positions(
            self.units
                .values()
                .filter(|unit| !unit.downed)
                .map(|unit| (unit.id, unit.position)),
        );
        occupancy
            .validate_route(path, unit)
            .map_err(|block| CommandRefusal::Occupied { block })?;
        let destination = path.last().copied().ok_or(CommandRefusal::InvalidPath)?;
        let actor = self
            .units
            .get_mut(&unit)
            .ok_or(CommandRefusal::UnknownUnit)?;
        actor.position = destination;
        let turn = actor.turn.as_mut().ok_or(CommandRefusal::NoTurn)?;
        turn.movement_left = turn.movement_left.saturating_sub(cost);
        Ok(())
    }

    fn apply_end_turn(&mut self, unit: UnitId) -> Result<(), CommandRefusal> {
        let _ = self.validate_actor(unit)?;
        let actor = self
            .units
            .get_mut(&unit)
            .ok_or(CommandRefusal::UnknownUnit)?;
        let turn = actor.turn.as_mut().ok_or(CommandRefusal::NoTurn)?;
        turn.movement_left = 0;
        turn.acted = true;
        Ok(())
    }

    fn apply_strike(&mut self, unit: UnitId, target: UnitId) -> Result<(), CommandRefusal> {
        let actor = self.validate_actor(unit)?;
        let turn = actor.turn.ok_or(CommandRefusal::NoTurn)?;
        if turn.acted {
            return Err(CommandRefusal::ActionAlreadySpent);
        }
        let target_unit = self
            .units
            .get(&target)
            .ok_or(CommandRefusal::UnknownTarget { target })?;
        if target_unit.downed {
            return Err(CommandRefusal::TargetDowned { target });
        }
        if !actor.faction.is_hostile_to(target_unit.faction) {
            return Err(CommandRefusal::TargetNotHostile { target });
        }
        if !self
            .arena
            .links
            .contains(&(actor.position, target_unit.position))
            || !self
                .arena
                .links
                .contains(&(target_unit.position, actor.position))
        {
            return Err(CommandRefusal::TargetOutOfMeleeReach { target });
        }
        let target_lattice =
            target_unit
                .lattice
                .as_ref()
                .ok_or(CommandRefusal::MissingUnitData {
                    unit: target,
                    data: UnitData::Lattice,
                })?;
        let count = resolve_incoming(&target_lattice.state, self.rules.strike_disable_count);
        self.units
            .get_mut(&unit)
            .and_then(|unit| unit.turn.as_mut())
            .ok_or(CommandRefusal::NoTurn)?
            .acted = true;
        self.events.push(CombatEvent::Strike {
            attacker: unit,
            target,
        });
        if count == 0 {
            self.events.push(CombatEvent::DamagePrevented {
                source: unit,
                target,
                amount: self.rules.strike_disable_count,
            });
        } else {
            self.pending = PendingDecision::ChooseDisables {
                decider: target,
                count,
                source: unit,
            };
            self.events.push(CombatEvent::DecisionOpened {
                decider: target,
                source: unit,
                count,
            });
        }
        Ok(())
    }

    fn apply_choose_disables(
        &mut self,
        unit: UnitId,
        cells: &[LatticeCoord],
    ) -> Result<(), CommandRefusal> {
        let PendingDecision::ChooseDisables {
            decider,
            count,
            source,
        } = self.pending
        else {
            return Err(CommandRefusal::NoPendingDecision);
        };
        if decider != unit {
            return Err(CommandRefusal::WrongDecisionUnit { expected: decider });
        }
        let lattice = self
            .units
            .get(&unit)
            .and_then(|unit| unit.lattice.as_ref())
            .ok_or(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Lattice,
            })?;
        let live = lattice
            .spec
            .cells()
            .filter(|(coord, _)| !lattice.state.is_disabled(*coord))
            .count();
        let owed = usize::from(count).min(live);
        if cells.len() != owed {
            return Err(CommandRefusal::WrongDisableCount {
                expected: u32::try_from(owed).unwrap_or(u32::MAX),
                actual: u32::try_from(cells.len()).unwrap_or(u32::MAX),
            });
        }
        let mut seen = BTreeSet::new();
        for &cell in cells {
            if lattice.spec.get(cell).is_none() {
                return Err(CommandRefusal::CellOutsideLattice { cell });
            }
            if !seen.insert(cell) {
                return Err(CommandRefusal::DuplicateCell { cell });
            }
            if lattice.state.is_disabled(cell) {
                return Err(CommandRefusal::CellAlreadyDisabled { cell });
            }
        }
        let actor = self
            .units
            .get_mut(&unit)
            .ok_or(CommandRefusal::UnknownUnit)?;
        let lattice = actor
            .lattice
            .as_mut()
            .ok_or(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Lattice,
            })?;
        let broken = apply_disables(&mut lattice.state, cells);
        self.pending = PendingDecision::None;
        self.events.push(CombatEvent::HexesDisabled {
            source,
            target: unit,
            cells: cells.to_vec(),
        });
        for record in broken {
            self.events.push(CombatEvent::EnchantmentBroken {
                unit,
                spell: None,
                burned_mana: record.burned_mana,
                trigger: record.trigger,
            });
        }
        let all_disabled = lattice
            .spec
            .cells()
            .all(|(coord, _)| lattice.state.is_disabled(coord));
        if all_disabled {
            actor.downed = true;
            actor.turn = None;
            self.events.push(CombatEvent::Downed { unit });
            self.remove_from_order(unit);
            self.detect_outcome();
        }
        Ok(())
    }

    fn apply_channel(&mut self, unit: UnitId) -> Result<(), CommandRefusal> {
        let actor = self.validate_actor(unit)?;
        let turn = actor.turn.ok_or(CommandRefusal::NoTurn)?;
        if turn.acted {
            return Err(CommandRefusal::ActionAlreadySpent);
        }
        let lattice = actor
            .lattice
            .as_ref()
            .ok_or(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Lattice,
            })?;
        for (_, kind) in lattice.spec.cells() {
            if let CellKind::Gem { element } = kind {
                if self.elements.name(element).is_none() {
                    return Err(CommandRefusal::MissingCombatData {
                        data: CombatData::ElementCatalog,
                    });
                }
            }
        }

        let actor = self
            .units
            .get_mut(&unit)
            .ok_or(CommandRefusal::UnknownUnit)?;
        let lattice = actor
            .lattice
            .as_mut()
            .ok_or(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Lattice,
            })?;
        let restored_by_id = channel(&mut lattice.state, &lattice.spec, &lattice.stats);
        let restored = restored_by_id
            .into_iter()
            .filter_map(|(element, amount)| {
                self.elements
                    .name(element)
                    .map(|name| (name.to_owned(), amount))
            })
            .collect::<BTreeMap<_, _>>();
        actor.turn.as_mut().ok_or(CommandRefusal::NoTurn)?.acted = true;
        for (element, amount) in &restored {
            *self
                .metrics
                .channelled_mana
                .entry(element.clone())
                .or_default() += u32::from(*amount);
        }
        self.events.push(CombatEvent::Channelled { unit, restored });
        Ok(())
    }

    fn advance_if_finished(&mut self) {
        if self.pending.is_open() {
            return;
        }
        let Some(unit) = self.current() else {
            return;
        };
        let finished = self.units.get(&unit).is_some_and(|actor| {
            !actor.busy
                && actor
                    .turn
                    .is_some_and(|turn| turn.acted && turn.movement_left == 0)
        });
        if !finished {
            return;
        }
        if let Some(actor) = self.units.get_mut(&unit) {
            actor.turn = None;
        }
        self.metrics.record_turn(unit);
        if !self.order.is_empty() {
            self.current = self.current.saturating_add(1);
            if self.current >= self.order.len() {
                self.current = 0;
                self.round = self.round.saturating_add(1);
            }
        }
        self.grant_current_turn();
        self.events.push(CombatEvent::TurnAdvanced {
            unit,
            next: self.current(),
            round: self.round,
        });
    }

    fn remove_from_order(&mut self, unit: UnitId) {
        let Some(index) = self.order.iter().position(|candidate| *candidate == unit) else {
            return;
        };
        self.order.remove(index);
        if self.order.is_empty() {
            self.current = 0;
        } else if index < self.current {
            self.current = self.current.saturating_sub(1);
        } else if self.current >= self.order.len() {
            self.current = 0;
        }
    }

    fn detect_outcome(&mut self) {
        let player_alive = self
            .units
            .values()
            .any(|unit| unit.faction == Faction::Player && !unit.downed);
        let hostile_alive = self
            .units
            .values()
            .any(|unit| unit.faction == Faction::Hostile && !unit.downed);
        let outcome = if !player_alive {
            Some(EncounterOutcome::Defeat)
        } else if !hostile_alive {
            Some(EncounterOutcome::Victory)
        } else {
            None
        };
        if let Some(outcome) = outcome {
            self.outcome = Some(outcome);
            self.events.push(CombatEvent::EncounterResolved { outcome });
        }
    }

    fn grant_current_turn(&mut self) {
        let Some(current) = self.current() else {
            return;
        };
        if let Some(actor) = self.units.get_mut(&current) {
            actor.turn = Some(Turn {
                movement_left: self.rules.movement_per_turn,
                acted: false,
            });
        }
    }

    /// Deterministic fingerprint of the complete serializable authority state.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        fingerprint(b"hex-combat-authority-v1", self)
    }
}

/// Who supplies commands in a deterministic case.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerInput {
    /// Repeatable scripted commands issued under one seat.
    Scripted(PlayerSeat),
}

/// Bounds for one deterministic run.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunBounds {
    /// Maximum commands applied before typed no-progress termination.
    pub max_commands: u32,
}

/// Complete pure simulation fixture.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatCase {
    /// Stable case identity.
    pub name: String,
    /// Frozen rules profile.
    pub rules: RulesProfile,
    /// Frozen spatial and observation input.
    pub arena: ArenaSnapshot,
    /// Frozen element display names.
    pub elements: ElementNames,
    /// Frozen roster.
    pub units: Vec<CombatUnit>,
    /// Typed command producer per unit.
    pub controllers: BTreeMap<UnitId, ControllerInput>,
    /// Run bound.
    pub bounds: RunBounds,
}

impl CombatCase {
    /// Runs a bounded canonical case without Bevy App, ECS schedule, or wall clock.
    pub fn run(&self) -> Result<CombatRunSnapshot, String> {
        let mut state = CombatState::start(
            self.rules.clone(),
            self.arena.clone(),
            self.elements.clone(),
            self.units.clone(),
        )?;
        for _ in 0..self.bounds.max_commands {
            if state.outcome.is_some() {
                break;
            }
            let Some(current) = state.current() else {
                break;
            };
            let seat = match self.controllers.get(&current) {
                Some(ControllerInput::Scripted(seat)) => *seat,
                None => {
                    return Err(format!(
                        "case {} has no controller for current unit {current:?}",
                        self.name
                    ));
                }
            };
            let issued = IssuedCommand {
                seat,
                command: GameCommand::EndTurn { unit: current },
            };
            if let Err(refusal) = state.apply(issued) {
                return Err(format!(
                    "case {} scripted command was refused: {refusal:?}",
                    self.name
                ));
            }
        }
        Ok(CombatRunSnapshot::from_state(self.name.clone(), state))
    }
}

/// Typed reason a bounded run stopped.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatTermination {
    /// Combat reached a retained-world outcome.
    Outcome(EncounterOutcome),
    /// The command bound was reached without progress to an outcome.
    BoundedNoProgress {
        /// Authoritative turns completed.
        completed_turns: u32,
        /// Current consecutive no-progress turns.
        no_progress_streak: u32,
    },
}

/// Canonical turn-order projection.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TurnStateSnapshot {
    /// Stable initiative order.
    pub order: Vec<UnitId>,
    /// Current actor.
    pub current: Option<UnitId>,
    /// Zero-based round.
    pub round: u32,
    /// Current turn state.
    pub active: Option<Turn>,
}

/// Canonical per-lattice totals.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LatticeSnapshot {
    /// Stable unit identity.
    pub unit: UnitId,
    /// Total gem mana.
    pub total_mana: u32,
    /// Total locked mana.
    pub locked_mana: u32,
}

/// Complete deterministic simulation artifact.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatRunSnapshot {
    /// Case identity.
    pub case: String,
    /// Canonical aggregate metrics.
    pub summary: CombatMetrics,
    /// Fingerprint of the complete final authority state.
    pub state_fingerprint: u64,
    /// Fingerprint of accepted command order.
    pub command_fingerprint: u64,
    /// Number of canonical events.
    pub transcript_event_count: usize,
    /// Fingerprint of the event transcript.
    pub transcript_fingerprint: u64,
    /// Outcome or explicit bound.
    pub termination: CombatTermination,
    /// Final turn facts.
    pub turn: TurnStateSnapshot,
    /// Final lattice totals.
    pub lattices: Vec<LatticeSnapshot>,
    /// Final exact positions.
    pub positions: BTreeMap<UnitId, TilePos>,
    /// Complete final state for field-level equality diagnostics.
    pub state: CombatState,
}

impl CombatRunSnapshot {
    fn from_state(case: String, state: CombatState) -> Self {
        let current = state.current();
        let active = current
            .and_then(|unit| state.units.get(&unit))
            .and_then(|unit| unit.turn);
        let turn = TurnStateSnapshot {
            order: state.order.clone(),
            current,
            round: state.round,
            active,
        };
        let positions = state
            .units
            .iter()
            .map(|(&unit, actor)| (unit, actor.position))
            .collect();
        let lattices = state
            .units
            .iter()
            .filter_map(|(&unit, actor)| {
                actor.lattice.as_ref().map(|lattice| LatticeSnapshot {
                    unit,
                    total_mana: lattice.state.total_gem_mana(),
                    locked_mana: lattice.state.total_locked_mana(),
                })
            })
            .collect();
        let termination = state.outcome.map_or(
            CombatTermination::BoundedNoProgress {
                completed_turns: state.metrics.turns,
                no_progress_streak: state.metrics.no_progress_current,
            },
            CombatTermination::Outcome,
        );
        Self {
            case,
            summary: state.metrics.clone(),
            state_fingerprint: state.fingerprint(),
            command_fingerprint: fingerprint(b"hex-combat-commands-v1", &state.commands),
            transcript_event_count: state.events.len(),
            transcript_fingerprint: fingerprint(b"hex-combat-transcript-v1", &state.events),
            termination,
            turn,
            lattices,
            positions,
            state,
        }
    }
}

fn fingerprint(domain: &[u8], value: &impl Serialize) -> u64 {
    let mut bytes = domain.to_vec();
    if serde_json::to_writer(&mut bytes, value).is_err() {
        bytes.extend_from_slice(b"<serialization-error>");
    }
    xxh3_64(&bytes)
}

#[cfg(test)]
mod tests {
    use hex_core::{ElementId, HexCoord, LatticeCoord};
    use hex_lattice::{apply_cast, castable, Casting, FusionTable, Requirement, SpellTable};

    use super::*;

    const LEVEL: i32 = 1;

    fn position(q: i32, r: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), LEVEL)
    }

    fn corridor(length: i32) -> ArenaSnapshot {
        let surfaces = (0..length).map(|q| position(q, 0)).collect::<Vec<_>>();
        let links = surfaces
            .windows(2)
            .flat_map(|edge| match edge {
                [a, b] => vec![(*a, *b), (*b, *a)],
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        ArenaSnapshot::new(surfaces.clone(), links)
            .expect("fixture arena")
            .with_observation(Faction::Player, surfaces.clone())
            .with_observation(Faction::Hostile, surfaces)
    }

    fn rules(budget: u32) -> RulesProfile {
        RulesProfile::new(format!("{budget}-step"), budget).expect("fixture rules")
    }

    fn state(units: Vec<CombatUnit>, budget: u32) -> CombatState {
        CombatState::start(rules(budget), corridor(5), ElementNames::default(), units)
            .expect("fixture state")
    }

    #[test]
    fn movement_uses_explicit_links_and_exact_occupancy() {
        let mut sim = state(
            vec![
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                ),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(2, 0),
                    10,
                ),
            ],
            4,
        );
        let before = sim.clone();
        let refusal = sim
            .apply(IssuedCommand {
                seat: PlayerSeat(0),
                command: GameCommand::MoveAlong {
                    unit: UnitId(0),
                    path: vec![position(0, 0), position(1, 0), position(2, 0)],
                },
            })
            .expect_err("occupied destination must refuse");
        assert!(matches!(refusal, CommandRefusal::Occupied { .. }));
        assert_eq!(sim.units, before.units);
        assert_eq!(sim.metrics.refused_commands, 1);
        assert_eq!(sim.events.len(), 1);
    }

    struct ChannelTables {
        fire: ElementId,
    }

    impl FusionTable for ChannelTables {
        fn recipe(&self, _output: ElementId) -> Option<Vec<Requirement>> {
            None
        }
    }

    impl SpellTable for ChannelTables {
        fn requirements(&self, _spell: hex_core::SpellId) -> Vec<Requirement> {
            vec![Requirement {
                element: self.fire,
                mana: 2,
            }]
        }

        fn casting(&self, _spell: hex_core::SpellId) -> Casting {
            Casting::Evocation
        }
    }

    fn depleted_lattice() -> (ElementId, LatticeSpec, LatticeState, LatticeStats) {
        let fire = ElementId(0);
        let spell = LatticeCoord::ORIGIN;
        let [gem, ..] = spell.neighbors();
        let spec = LatticeSpec::default()
            .with(
                spell,
                CellKind::Spell {
                    spell: hex_core::SpellId(0),
                },
            )
            .with(gem, CellKind::Gem { element: fire });
        let stats = LatticeStats::new(BTreeMap::from([(fire, 3)]), BTreeMap::from([(fire, 2)]));
        let mut state = LatticeState::new(&spec, &stats);
        let tables = ChannelTables { fire };
        let plan = castable(&spec, &state, spell, &tables).expect("fixture cast");
        assert!(apply_cast(&mut state, &plan, &tables));
        (fire, spec, state, stats)
    }

    #[test]
    fn channel_is_one_action_and_refusal_is_transactional() {
        let (fire, spec, lattice, stats) = depleted_lattice();
        let units = vec![
            CombatUnit::new(
                UnitId(0),
                PlayerSeat(0),
                Faction::Player,
                position(0, 0),
                20,
            )
            .with_lattice(spec, lattice, stats),
            CombatUnit::new(
                UnitId(1),
                PlayerSeat(0),
                Faction::Hostile,
                position(4, 0),
                10,
            ),
        ];
        let mut sim = CombatState::start(
            rules(4),
            corridor(5),
            ElementNames::new(BTreeMap::from([(fire, "Fire".to_owned())])),
            units,
        )
        .expect("fixture state");
        let command = IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::Channel { unit: UnitId(0) },
        };
        assert!(sim.apply(command.clone()).is_ok());
        let after_success = sim.clone();
        assert_eq!(
            sim.units
                .get(&UnitId(0))
                .and_then(|unit| unit.lattice.as_ref())
                .map(|lattice| lattice.state.total_gem_mana()),
            Some(3)
        );
        assert!(matches!(
            sim.apply(command),
            Err(CommandRefusal::ActionAlreadySpent)
        ));
        assert_eq!(sim.units, after_success.units);
        assert_eq!(sim.metrics.channels, 1);
        assert_eq!(sim.metrics.successful_commands, 1);
        assert_eq!(sim.metrics.refused_commands, 1);
        assert_eq!(sim.metrics.channelled_mana.get("Fire"), Some(&2));
    }

    #[test]
    fn bounded_case_runs_twice_to_complete_snapshot_equality() {
        let units = vec![
            CombatUnit::new(
                UnitId(0),
                PlayerSeat(0),
                Faction::Player,
                position(0, 0),
                20,
            ),
            CombatUnit::new(
                UnitId(1),
                PlayerSeat(0),
                Faction::Hostile,
                position(4, 0),
                10,
            ),
        ];
        let case = CombatCase {
            name: "two-by-two".to_owned(),
            rules: rules(4),
            arena: corridor(5),
            elements: ElementNames::default(),
            controllers: units
                .iter()
                .map(|unit| (unit.id, ControllerInput::Scripted(unit.seat)))
                .collect(),
            units,
            bounds: RunBounds { max_commands: 8 },
        };
        let first = case.run().expect("first run");
        let second = case.run().expect("second run");
        assert_eq!(first, second);
        assert_eq!(first.summary.turns, 8);
        assert_eq!(first.summary.no_progress_current, 8);
        assert!(matches!(
            first.termination,
            CombatTermination::BoundedNoProgress {
                completed_turns: 8,
                no_progress_streak: 8
            }
        ));
    }

    #[test]
    fn state_round_trip_preserves_the_next_reduction() {
        let mut original = state(
            vec![
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                ),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(4, 0),
                    10,
                ),
            ],
            3,
        );
        assert!(original
            .apply(IssuedCommand {
                seat: PlayerSeat(0),
                command: GameCommand::EndTurn { unit: UnitId(0) },
            })
            .is_ok());
        let encoded = serde_json::to_string(&original).expect("state serializes");
        let mut restored: CombatState = serde_json::from_str(&encoded).expect("state deserializes");
        let next = IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::EndTurn { unit: UnitId(1) },
        };
        assert!(original.apply(next.clone()).is_ok());
        assert!(restored.apply(next).is_ok());
        assert_eq!(restored, original);
    }

    #[test]
    fn strike_decision_downs_the_last_hostile_and_resolves_victory() {
        let cell = LatticeCoord::ORIGIN;
        let spec = LatticeSpec::default().with(cell, CellKind::Blank);
        let mut sim = CombatState::start(
            rules(4),
            corridor(2),
            ElementNames::default(),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                ),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(1, 0),
                    10,
                )
                .with_lattice(
                    spec.clone(),
                    LatticeState::new(&spec, &LatticeStats::default()),
                    LatticeStats::default(),
                ),
            ],
        )
        .expect("fixture state");
        assert!(sim
            .apply(IssuedCommand {
                seat: PlayerSeat(0),
                command: GameCommand::Strike {
                    unit: UnitId(0),
                    target: UnitId(1),
                },
            })
            .is_ok());
        assert!(matches!(
            sim.pending,
            PendingDecision::ChooseDisables {
                decider: UnitId(1),
                count: 1,
                source: UnitId(0)
            }
        ));
        assert!(sim
            .apply(IssuedCommand {
                seat: PlayerSeat(0),
                command: GameCommand::ChooseDisables {
                    unit: UnitId(1),
                    cells: vec![cell],
                },
            })
            .is_ok());
        assert_eq!(sim.outcome, Some(EncounterOutcome::Victory));
        assert!(sim.units.get(&UnitId(1)).is_some_and(|unit| unit.downed));
        assert!(sim.events.iter().any(|event| matches!(
            event,
            CombatEvent::EncounterResolved {
                outcome: EncounterOutcome::Victory
            }
        )));
    }
}
