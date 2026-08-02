//! Serializable combat state and its single command reducer.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use hex_core::{
    EffectEnd, EffectId, EffectPayload, Faction, GameCommand, IssuedCommand, LatticeCoord,
    PendingDecision, PersistentEffect, PlayerSeat, TilePos, Turn, UnitId, UnitOccupancy,
};
use hex_lattice::{
    apply_disables, castable, channel, resolve_incoming, restore, CastBlocked, CellKind,
    LatticeSpec, LatticeState, LatticeStats,
};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::{
    CastBlockReason, CombatData, CombatEvent, CommandRefusal, EncounterOutcome,
    FrozenCombatContent, FrozenEffect, FrozenTargeting, RestorationRefusal, UnitData,
};

/// Current pure rules-policy schema.
pub const RULES_PROFILE_VERSION: u16 = 1;

/// Implemented initiative policy.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitiativePolicy {
    /// One slot per active unit, fixed by authored initiative then stable unit id.
    FixedByInitiativeThenUnitId,
}

/// Implemented action-economy policy.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEconomyPolicy {
    /// A movement budget plus exactly one action.
    MovementAndOneAction,
}

/// Frozen, content-independent policy values used by combat.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RulesProfile {
    /// Serialized pure-policy schema.
    pub version: u16,
    /// Stable profile identity included in snapshots and reports.
    pub name: String,
    /// Typed initiative baseline.
    pub initiative: InitiativePolicy,
    /// Typed action-economy baseline.
    pub action_economy: ActionEconomyPolicy,
    /// Exact movement edges granted at the start of every turn.
    pub movement_per_turn: u32,
    /// Raw lattice disables opened by the placeholder melee strike.
    pub strike_disable_count: u16,
    /// Elevation levels required for one bonus target-range hex.
    pub levels_per_bonus_range: u32,
    /// Further round rollovers each Reveal tier remains known.
    pub reveal_rounds_per_tier: u32,
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
            version: RULES_PROFILE_VERSION,
            name,
            initiative: InitiativePolicy::FixedByInitiativeThenUnitId,
            action_economy: ActionEconomyPolicy::MovementAndOneAction,
            movement_per_turn,
            strike_disable_count: 1,
            levels_per_bonus_range: 5,
            reveal_rounds_per_tier: 1,
        })
    }

    /// Overrides the provisional strike count without changing command semantics.
    #[must_use]
    pub fn with_strike_disable_count(mut self, count: u16) -> Self {
        self.strike_disable_count = count;
        self
    }

    /// Overrides targeting and Reveal policy while retaining the shipping algorithms.
    #[must_use]
    pub fn with_cast_policy(
        mut self,
        levels_per_bonus_range: u32,
        reveal_rounds_per_tier: u32,
    ) -> Self {
        self.levels_per_bonus_range = levels_per_bonus_range;
        self.reveal_rounds_per_tier = reveal_rounds_per_tier;
        self
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != RULES_PROFILE_VERSION {
            return Err(format!(
                "rules profile version {} is unsupported; expected {RULES_PROFILE_VERSION}",
                self.version
            ));
        }
        if self.movement_per_turn == 0
            || self.strike_disable_count == 0
            || self.levels_per_bonus_range == 0
            || self.reveal_rounds_per_tier == 0
        {
            return Err(
                "pure combat rules require positive movement, strike, range, and Reveal values"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

/// Stable element names resolved before simulation starts.
///
/// IDs remain session-local; reports use these frozen names and never reach into the
/// live asset server while reducing a command.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ElementNames {
    by_id: BTreeMap<hex_core::ElementId, String>,
    spell_by_id: BTreeMap<hex_core::SpellId, String>,
}

impl ElementNames {
    /// Creates an explicit id-to-name table.
    #[must_use]
    pub fn new(by_id: BTreeMap<hex_core::ElementId, String>) -> Self {
        Self {
            by_id,
            spell_by_id: BTreeMap::new(),
        }
    }

    /// Attaches stable spell names used when a disabled cell breaks an enchantment.
    #[must_use]
    pub fn with_spells(
        mut self,
        spell_by_id: impl IntoIterator<Item = (hex_core::SpellId, String)>,
    ) -> Self {
        self.spell_by_id = spell_by_id.into_iter().collect();
        self
    }

    fn name(&self, id: hex_core::ElementId) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }

    fn spell_name(&self, id: hex_core::SpellId) -> Option<&str> {
        self.spell_by_id.get(&id).map(String::as_str)
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
    unit_links: BTreeMap<UnitId, BTreeSet<(TilePos, TilePos)>>,
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
            unit_links: BTreeMap::new(),
            observed: BTreeMap::new(),
        })
    }

    /// Overrides traversal edges for one stable unit.
    ///
    /// Runtime hosts publish body-specific footing here so a small unit's
    /// crawlspace does not authorize a larger unit to follow it.
    pub fn with_unit_links(
        mut self,
        unit: UnitId,
        links: impl IntoIterator<Item = (TilePos, TilePos)>,
    ) -> Result<Self, String> {
        let links = links.into_iter().collect::<BTreeSet<_>>();
        for (from, to) in &links {
            if from == to || !self.surfaces.contains(from) || !self.surfaces.contains(to) {
                return Err(format!(
                    "unit {unit:?} traversal names an invalid arena edge: {from:?} -> {to:?}"
                ));
            }
        }
        self.unit_links.insert(unit, links);
        Ok(self)
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

    fn links_for(&self, unit: UnitId) -> &BTreeSet<(TilePos, TilePos)> {
        self.unit_links.get(&unit).unwrap_or(&self.links)
    }

    fn validates_path(&self, unit: UnitId, path: &[TilePos]) -> bool {
        let links = self.links_for(unit);
        !path.is_empty()
            && path.iter().all(|position| self.surfaces.contains(position))
            && path
                .windows(2)
                .all(|edge| matches!(edge, [from, to] if links.contains(&(*from, *to))))
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

/// A validated exact-surface route awaiting domain movement completion.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CombatMotion {
    /// Full route, including the surface currently occupied.
    pub path: Vec<TilePos>,
    /// Last route index published as the unit's exact position.
    pub reached: usize,
}

/// A host projection disagreed with a committed domain route.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementProjectionError {
    /// The stable unit does not exist in this authority state.
    UnknownUnit(UnitId),
    /// The unit has no route awaiting progress.
    NoMovementInFlight(UnitId),
    /// The host published a surface outside the monotonic remainder of the route.
    RouteMismatch {
        /// Stable unit identity.
        unit: UnitId,
        /// Exact rejected surface.
        position: TilePos,
    },
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
    /// Domain movement in flight. Animation is a projection of this record.
    pub motion: Option<CombatMotion>,
    /// Whether the unit has left initiative.
    pub downed: bool,
    /// Optional lattice. Harnesses may omit it for movement-only combatants.
    pub lattice: Option<CombatLattice>,
}

/// Explicit ECS facts accepted at a content-dependent adapter boundary.
///
/// Reducer-covered commands never use this type. It exists for commands whose
/// authored spell/effect resolution still belongs to the Bevy host: the host
/// publishes the complete resulting domain facts back to the authority before any
/// later command may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatUnitProjection {
    /// Stable unit identity.
    pub id: UnitId,
    /// Exact occupied surface.
    pub position: TilePos,
    /// Current turn budget, if this unit owns one.
    pub turn: Option<Turn>,
    /// Domain command gate.
    pub busy: bool,
    /// Whether the unit has left initiative.
    pub downed: bool,
    /// Mutable lattice state. The fixed spec and stats remain frozen.
    pub lattice: Option<LatticeState>,
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
            motion: None,
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
    /// Frozen authored facts required by active-combat casts.
    pub content: FrozenCombatContent,
    /// Unit records in stable-id order.
    pub units: BTreeMap<UnitId, CombatUnit>,
    /// Initiative order in stable identities.
    pub order: Vec<UnitId>,
    /// Current order index.
    current: usize,
    /// Zero-based completed round count.
    pub round: u32,
    /// Revived units waiting for an exact round boundary before rejoining initiative.
    pub pending_revivals: BTreeMap<UnitId, u32>,
    /// Running persistent effects in allocation order.
    pub effects: BTreeMap<EffectId, PersistentEffect>,
    /// Next monotonic persistent-effect identity.
    next_effect_id: u64,
    /// Complete-lattice Reveal expiry keyed by viewer and subject.
    pub reveals: BTreeMap<(Faction, UnitId), u32>,
    /// Canonical aggregate summary.
    pub metrics: CombatMetrics,
    /// One defender choice suspending further simulation commands.
    pub pending: PendingDecision,
    /// A host-resolved spell transaction still owns completion authority.
    ///
    /// Area spell effects and terrain acknowledgements are resolved by the Bevy host,
    /// but defender answers still reduce here. This hold prevents the first answer
    /// from advancing the turn or settling the encounter before the host publishes
    /// every remaining obligation.
    #[serde(default)]
    resolution_held: bool,
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
        Self::start_with_content(
            rules,
            arena,
            elements,
            FrozenCombatContent::default(),
            units,
        )
    }

    /// Starts combat with immutable active-combat content.
    pub fn start_with_content(
        rules: RulesProfile,
        arena: ArenaSnapshot,
        elements: ElementNames,
        content: FrozenCombatContent,
        units: impl IntoIterator<Item = CombatUnit>,
    ) -> Result<Self, String> {
        Self::start_with_content_and_session(
            rules,
            arena,
            elements,
            content,
            units,
            PendingDecision::None,
            BTreeMap::new(),
        )
    }

    /// Starts combat while adopting already-published session facts.
    ///
    /// Runtime normally supplies no pending choice or revival at entry. This explicit
    /// constructor exists for restored sessions and fixture-owned contract state; the
    /// facts are validated against the same frozen roster before becoming authority.
    pub fn start_with_session(
        rules: RulesProfile,
        arena: ArenaSnapshot,
        elements: ElementNames,
        units: impl IntoIterator<Item = CombatUnit>,
        pending: PendingDecision,
        pending_revivals: BTreeMap<UnitId, u32>,
    ) -> Result<Self, String> {
        Self::start_with_content_and_session(
            rules,
            arena,
            elements,
            FrozenCombatContent::default(),
            units,
            pending,
            pending_revivals,
        )
    }

    /// Starts combat while adopting frozen content and already-published session facts.
    pub fn start_with_content_and_session(
        rules: RulesProfile,
        arena: ArenaSnapshot,
        elements: ElementNames,
        content: FrozenCombatContent,
        units: impl IntoIterator<Item = CombatUnit>,
        pending: PendingDecision,
        pending_revivals: BTreeMap<UnitId, u32>,
    ) -> Result<Self, String> {
        rules.validate()?;
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
            .filter(|unit| !unit.downed && !pending_revivals.contains_key(&unit.id))
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
            content,
            units: by_id,
            order,
            current: 0,
            round: 0,
            pending_revivals,
            effects: BTreeMap::new(),
            next_effect_id: 0,
            reveals: BTreeMap::new(),
            metrics: CombatMetrics::default(),
            pending,
            resolution_held: false,
            commands: Vec::new(),
            events: Vec::new(),
            outcome: None,
        };
        state.validate_session_facts()?;
        state.grant_current_turn();
        Ok(state)
    }

    fn validate_session_facts(&self) -> Result<(), String> {
        let known = |unit| self.units.contains_key(&unit);
        match self.pending {
            PendingDecision::None => {}
            PendingDecision::ChooseDisables {
                decider, source, ..
            } => {
                if !known(decider) || !known(source) {
                    return Err(
                        "pending disable decision names a unit outside the roster".to_owned()
                    );
                }
            }
            PendingDecision::ChooseRestores {
                decider, target, ..
            } => {
                if !known(decider) || !known(target) {
                    return Err(
                        "pending restoration decision names a unit outside the roster".to_owned(),
                    );
                }
            }
        }
        if self
            .pending_revivals
            .keys()
            .any(|unit| !self.units.contains_key(unit))
        {
            return Err("pending revival names a unit outside the roster".to_owned());
        }
        Ok(())
    }

    /// Stable identity currently allowed to act.
    #[must_use]
    pub fn current(&self) -> Option<UnitId> {
        self.order.get(self.current).copied()
    }

    /// Begins one host-owned spell-resolution transaction.
    ///
    /// The hold is deliberately separate from [`PendingDecision`]: the public
    /// decision names only the one answer currently owed, while terrain and later
    /// occupants may still be unresolved after that answer clears.
    pub fn begin_external_resolution(&mut self) -> Result<(), String> {
        if self.resolution_held {
            return Err("combat authority already holds an external resolution".to_owned());
        }
        self.resolution_held = true;
        Ok(())
    }

    /// Releases a complete host-owned spell resolution and resumes authority once.
    pub fn finish_external_resolution(&mut self) -> Result<(), String> {
        if !self.resolution_held {
            return Err("combat authority has no external resolution to finish".to_owned());
        }
        self.resolution_held = false;
        self.detect_outcome();
        self.advance_if_finished();
        Ok(())
    }

    /// Whether a host-owned spell transaction still blocks ordinary authority.
    #[must_use]
    pub const fn external_resolution_is_held(&self) -> bool {
        self.resolution_held
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

    /// Adopts one complete, explicit projection after a content-dependent host
    /// adapter has resolved a command.
    ///
    /// This is not a second command reducer: it accepts no intent and derives no
    /// gameplay rule. The adapter must publish every mutable fact, and malformed or
    /// partial projections fail closed before replacing authority state.
    pub fn adopt_projection(
        &mut self,
        order: Vec<UnitId>,
        current: Option<UnitId>,
        round: u32,
        pending: PendingDecision,
        pending_revivals: BTreeMap<UnitId, u32>,
        units: impl IntoIterator<Item = CombatUnitProjection>,
    ) -> Result<(), String> {
        let checkpoint = self.clone();
        let result = self.adopt_projection_inner(
            order,
            current,
            round,
            pending,
            pending_revivals,
            units.into_iter().collect(),
        );
        if result.is_err() {
            *self = checkpoint;
        }
        result
    }

    fn adopt_projection_inner(
        &mut self,
        order: Vec<UnitId>,
        current: Option<UnitId>,
        round: u32,
        pending: PendingDecision,
        pending_revivals: BTreeMap<UnitId, u32>,
        units: Vec<CombatUnitProjection>,
    ) -> Result<(), String> {
        let projections = units
            .into_iter()
            .map(|projection| (projection.id, projection))
            .collect::<BTreeMap<_, _>>();
        if projections.len() != self.units.len()
            || projections
                .keys()
                .any(|unit| !self.units.contains_key(unit))
        {
            return Err("adapter projection does not name the exact frozen roster".to_owned());
        }
        let mut ordered = BTreeSet::new();
        for unit in &order {
            let projection = projections
                .get(unit)
                .ok_or_else(|| format!("adapter order names unknown unit {unit:?}"))?;
            if projection.downed || !ordered.insert(*unit) {
                return Err(format!(
                    "adapter order duplicates or retains downed unit {unit:?}"
                ));
            }
        }
        let current_index = match current {
            Some(unit) => order
                .iter()
                .position(|candidate| *candidate == unit)
                .ok_or_else(|| format!("adapter current unit {unit:?} is outside its order"))?,
            None if order.is_empty() => 0,
            None => return Err("adapter omitted current unit for a non-empty order".to_owned()),
        };
        let mut positions = BTreeSet::new();
        for projection in projections.values() {
            if !self.arena.surfaces.contains(&projection.position) {
                return Err(format!(
                    "adapter unit {:?} stands outside the frozen arena at {:?}",
                    projection.id, projection.position
                ));
            }
            if !positions.insert(projection.position) {
                return Err(format!(
                    "adapter projection duplicates exact surface {:?}",
                    projection.position
                ));
            }
            let in_order = ordered.contains(&projection.id);
            let awaiting_revival = pending_revivals.contains_key(&projection.id);
            if projection.downed {
                if in_order || awaiting_revival {
                    return Err(format!(
                        "downed adapter unit {:?} remains active or awaits revival",
                        projection.id
                    ));
                }
            } else if in_order == awaiting_revival {
                return Err(format!(
                    "live adapter unit {:?} must be in initiative or await revival, exclusively",
                    projection.id
                ));
            }
            if projection.turn.is_some() != (current == Some(projection.id)) {
                return Err(format!(
                    "adapter turn marker disagrees for unit {:?}",
                    projection.id
                ));
            }
        }

        for (id, projection) in projections {
            let actor = self
                .units
                .get_mut(&id)
                .ok_or_else(|| format!("adapter projected unknown unit {id:?}"))?;
            match (&mut actor.lattice, projection.lattice) {
                (Some(lattice), Some(state)) => lattice.state = state,
                (lattice @ Some(_), None) => *lattice = None,
                (None, None) => {}
                _ => {
                    return Err(format!(
                        "adapter introduced a lattice for unit {id:?} without frozen facts"
                    ));
                }
            }
            actor.position = projection.position;
            actor.turn = projection.turn;
            actor.busy = projection.busy;
            if !projection.busy {
                actor.motion = None;
            }
            actor.downed = projection.downed;
        }
        self.order = order;
        self.current = current_index;
        self.round = round;
        self.pending = pending;
        self.pending_revivals = pending_revivals;
        Ok(())
    }

    /// Settles terminal state after an external adapter projection is complete.
    ///
    /// Hosts call this after the current command drain so a refusal already in that
    /// drain retains its canonical ordering ahead of the terminal outcome.
    pub fn settle_outcome(&mut self) {
        self.detect_outcome();
    }

    /// Records a successfully resolved adapter command in the canonical transcript.
    pub fn record_adapter_success(&mut self, issued: IssuedCommand) {
        self.metrics.record_success(&issued.command);
        self.commands.push(issued);
    }

    /// Records a refused adapter command without mutating domain state.
    pub fn record_adapter_refusal(&mut self, issued: IssuedCommand, refusal: CommandRefusal) {
        self.metrics.record_refusal();
        self.events.push(CombatEvent::CommandRefused {
            command: issued.command,
            refusal,
        });
    }

    /// Publishes one reached surface for a validated in-flight route.
    ///
    /// Runtime movement clocks and deterministic harnesses call this boundary;
    /// presentation components never do. The update is monotonic and fails closed
    /// if a caller skips off the committed route.
    pub fn reach_movement(
        &mut self,
        unit: UnitId,
        position: TilePos,
    ) -> Result<(), MovementProjectionError> {
        let actor = self
            .units
            .get_mut(&unit)
            .ok_or(MovementProjectionError::UnknownUnit(unit))?;
        let complete = {
            let motion = actor
                .motion
                .as_mut()
                .ok_or(MovementProjectionError::NoMovementInFlight(unit))?;
            let Some(next) = motion
                .path
                .get(motion.reached..)
                .and_then(|tail| tail.iter().position(|step| *step == position))
            else {
                return Err(MovementProjectionError::RouteMismatch { unit, position });
            };
            motion.reached = motion.reached.saturating_add(next);
            motion.reached.saturating_add(1) >= motion.path.len()
        };
        actor.position = position;
        if complete {
            actor.motion = None;
            actor.busy = false;
            self.advance_if_finished();
        }
        Ok(())
    }

    /// Settles an in-flight route at its validated endpoint.
    pub fn complete_movement(&mut self, unit: UnitId) -> Result<(), MovementProjectionError> {
        let destination = self
            .units
            .get(&unit)
            .ok_or(MovementProjectionError::UnknownUnit(unit))?
            .motion
            .as_ref()
            .and_then(|motion| motion.path.last())
            .copied()
            .ok_or(MovementProjectionError::NoMovementInFlight(unit))?;
        self.reach_movement(unit, destination)
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
        if self.resolution_held && !self.pending.is_open() {
            return Err(CommandRefusal::Busy);
        }
        let required_answer = match self.pending {
            PendingDecision::None => None,
            PendingDecision::ChooseDisables { decider, .. } => Some((decider, false)),
            PendingDecision::ChooseRestores { decider, .. } => Some((decider, true)),
        };
        if let Some((decider, restores)) = required_answer {
            let matches = if restores {
                matches!(
                    issued.command,
                    GameCommand::ChooseRestores {
                        unit: answerer,
                        ..
                    } if answerer == decider
                )
            } else {
                matches!(
                    issued.command,
                    GameCommand::ChooseDisables {
                        unit: answerer,
                        ..
                    } if answerer == decider
                )
            };
            if !matches {
                return Err(CommandRefusal::DecisionPending { decider });
            }
        }
        match &issued.command {
            GameCommand::MoveAlong { unit, path } => self.apply_move(*unit, path),
            GameCommand::Strike { unit, target } => self.apply_strike(*unit, *target),
            GameCommand::EndTurn { unit } => self.apply_end_turn(*unit),
            GameCommand::Channel { unit } => self.apply_channel(*unit),
            GameCommand::ChooseDisables { unit, cells } => self.apply_choose_disables(*unit, cells),
            GameCommand::ChooseRestores {
                unit,
                target,
                cells,
            } => self.apply_choose_restores(*unit, *target, cells),
            GameCommand::Cast {
                unit,
                spell,
                target,
                facing,
                ..
            } => self.apply_cast(*unit, spell, *target, *facing),
            // Exploration verbs remain explicit host adapters. They are outside the
            // active-combat authority and are not evidence for a fight simulation.
            GameCommand::MoveParty { .. } => Err(CommandRefusal::PartyMovementUnavailable),
            GameCommand::Rest { .. } => Err(CommandRefusal::RestUnavailable),
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
        if path.len() < 2
            || path.first().copied() != Some(actor.position)
            || !self.arena.validates_path(unit, path)
        {
            return Err(CommandRefusal::InvalidPath);
        }
        let cost = u32::try_from(path.len().saturating_sub(1)).unwrap_or(u32::MAX);
        let remaining = actor.turn.map_or(0, |turn| turn.movement_left);
        if cost > remaining {
            return Err(CommandRefusal::MovementBudgetExceeded { cost, remaining });
        }
        let occupancy = UnitOccupancy::from_positions(self.units.values().flat_map(|actor| {
            std::iter::once((actor.id, actor.position)).chain(
                actor
                    .motion
                    .iter()
                    .flat_map(|motion| motion.path.iter().copied())
                    .map(|position| (actor.id, position)),
            )
        }));
        occupancy
            .validate_route(path, unit)
            .map_err(|block| CommandRefusal::Occupied { block })?;
        let actor = self
            .units
            .get_mut(&unit)
            .ok_or(CommandRefusal::UnknownUnit)?;
        let turn = actor.turn.as_mut().ok_or(CommandRefusal::NoTurn)?;
        turn.movement_left = turn.movement_left.saturating_sub(cost);
        actor.busy = true;
        actor.motion = Some(CombatMotion {
            path: path.to_vec(),
            reached: 0,
        });
        Ok(())
    }

    fn apply_end_turn(&mut self, unit: UnitId) -> Result<(), CommandRefusal> {
        let actor = self.units.get(&unit).ok_or(CommandRefusal::UnknownUnit)?;
        if actor.downed {
            return Err(CommandRefusal::ActingUnitDowned { unit });
        }
        if self.current() != Some(unit) {
            return Err(CommandRefusal::NotCurrentTurn {
                current: self.current(),
            });
        }
        if actor.turn.is_none() {
            return Err(CommandRefusal::NoTurn);
        }
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
        let (turn, actor_position, actor_faction) = {
            let actor = self.validate_actor(unit)?;
            (
                actor.turn.ok_or(CommandRefusal::NoTurn)?,
                actor.position,
                actor.faction,
            )
        };
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
        if !actor_faction.is_hostile_to(target_unit.faction) {
            return Err(CommandRefusal::TargetNotHostile { target });
        }
        let links = self.arena.links_for(unit);
        if !links.contains(&(actor_position, target_unit.position))
            || !links.contains(&(target_unit.position, actor_position))
        {
            return Err(CommandRefusal::TargetOutOfMeleeReach { target });
        }
        // Lattice-less units are playable but cannot take lattice damage. A strike is
        // still a successful action and presentation event, matching the runtime
        // adapter; it simply opens no unanswerable defender decision.
        let target_lattice = target_unit.lattice.as_ref();
        let target_has_lattice = target_lattice.is_some();
        let count = target_lattice.map_or(0, |lattice| {
            resolve_incoming(&lattice.state, self.rules.strike_disable_count)
        });
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
            if target_has_lattice {
                self.events.push(CombatEvent::DamagePrevented {
                    source: unit,
                    target,
                    amount: self.rules.strike_disable_count,
                });
            }
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
                spell: self.elements.spell_name(record.spell).map(str::to_owned),
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

    fn apply_cast(
        &mut self,
        unit: UnitId,
        spell_name: &str,
        target: TilePos,
        _facing: Option<hex_core::Sextant>,
    ) -> Result<(), CommandRefusal> {
        let (turn, actor_position, actor_faction) = {
            let actor = self.validate_actor(unit)?;
            (
                actor.turn.ok_or(CommandRefusal::NoTurn)?,
                actor.position,
                actor.faction,
            )
        };
        if turn.acted {
            return Err(CommandRefusal::ActionAlreadySpent);
        }
        let spell = self.content.spell(spell_name).cloned().ok_or_else(|| {
            CommandRefusal::UnknownSpell {
                spell: spell_name.to_owned(),
            }
        })?;
        match spell.targeting {
            FrozenTargeting::SelfOnly if target != actor_position => {
                return Err(CommandRefusal::TargetOutOfRange {
                    spell: spell_name.to_owned(),
                    target,
                });
            }
            FrozenTargeting::SelfOnly => {}
            FrozenTargeting::ExactSurface { range } => {
                let high_ground = actor_position.level.saturating_sub(target.level).max(0);
                let bonus = u32::try_from(high_ground)
                    .unwrap_or(u32::MAX)
                    .checked_div(self.rules.levels_per_bonus_range)
                    .unwrap_or_default();
                if actor_position.coord.distance(target.coord) > range.saturating_add(bonus) {
                    return Err(CommandRefusal::TargetOutOfRange {
                        spell: spell_name.to_owned(),
                        target,
                    });
                }
            }
        }
        if !self.arena.observes(actor_faction, target) {
            return Err(CommandRefusal::TargetUnobserved {
                spell: spell_name.to_owned(),
                target,
            });
        }
        let target_unit = self
            .units
            .values()
            .find(|candidate| candidate.position == target)
            .map(|candidate| (candidate.id, candidate.downed));
        let damages = spell.effects.iter().any(|effect| {
            matches!(
                effect,
                FrozenEffect::DisableHexes { .. } | FrozenEffect::Burn { .. }
            )
        });
        if let Some((target, true)) = target_unit {
            if damages {
                return Err(CommandRefusal::TargetDowned { target });
            }
        }
        let cell = {
            let lattice = self
                .units
                .get(&unit)
                .and_then(|actor| actor.lattice.as_ref())
                .ok_or(CommandRefusal::MissingUnitData {
                    unit,
                    data: UnitData::Lattice,
                })?;
            spell_cell(&lattice.spec, &lattice.state, spell.id).ok_or_else(|| {
                CommandRefusal::SpellNotInscribed {
                    spell: spell_name.to_owned(),
                }
            })?
        };
        let plan = {
            let lattice = self
                .units
                .get(&unit)
                .and_then(|actor| actor.lattice.as_ref())
                .ok_or(CommandRefusal::MissingUnitData {
                    unit,
                    data: UnitData::Lattice,
                })?;
            castable(&lattice.spec, &lattice.state, cell, &self.content).map_err(|blocked| {
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
        {
            let lattice = self
                .units
                .get_mut(&unit)
                .and_then(|actor| actor.lattice.as_mut())
                .ok_or(CommandRefusal::MissingUnitData {
                    unit,
                    data: UnitData::Lattice,
                })?;
            if !hex_lattice::apply_cast(&mut lattice.state, &plan, &self.content) {
                return Err(CommandRefusal::CastPlanStale {
                    spell: spell_name.to_owned(),
                });
            }
        }
        self.events.push(CombatEvent::Cast {
            caster: unit,
            spell: spell_name.to_owned(),
            target,
        });

        for effect in spell.effects {
            match effect {
                FrozenEffect::DisableHexes { count } => {
                    let Some((target, _)) = target_unit else {
                        continue;
                    };
                    let Some(lattice) = self
                        .units
                        .get(&target)
                        .and_then(|target| target.lattice.as_ref())
                    else {
                        continue;
                    };
                    let landed = resolve_incoming(&lattice.state, count);
                    let prevented = count.saturating_sub(landed);
                    if prevented > 0 {
                        self.events.push(CombatEvent::DamagePrevented {
                            source: unit,
                            target,
                            amount: prevented,
                        });
                    }
                    if landed > 0 {
                        self.pending = PendingDecision::ChooseDisables {
                            decider: target,
                            count: landed,
                            source: unit,
                        };
                        self.events.push(CombatEvent::DecisionOpened {
                            decider: target,
                            source: unit,
                            count: landed,
                        });
                    }
                }
                FrozenEffect::Burn { turns } => {
                    let Some((target, _)) = target_unit else {
                        continue;
                    };
                    if turns == 0
                        || self
                            .units
                            .get(&target)
                            .and_then(|target| target.lattice.as_ref())
                            .is_none()
                    {
                        continue;
                    }
                    let id = EffectId(self.next_effect_id);
                    self.next_effect_id = self.next_effect_id.saturating_add(1);
                    self.effects.insert(
                        id,
                        PersistentEffect {
                            source: unit,
                            target,
                            payload: EffectPayload::Burn,
                            start: self.round,
                            end: EffectEnd::AfterTurns(turns),
                            ticks: 0,
                        },
                    );
                    self.events.push(CombatEvent::BurnApplied {
                        source: unit,
                        target,
                        turns,
                    });
                }
                FrozenEffect::RestoreHexes { count } => {
                    let Some((target, _)) = target_unit else {
                        continue;
                    };
                    let disabled = self
                        .units
                        .get(&target)
                        .and_then(|target| target.lattice.as_ref())
                        .map_or(0, |lattice| {
                            lattice
                                .spec
                                .cells()
                                .filter(|(coord, _)| lattice.state.is_disabled(*coord))
                                .count()
                        });
                    let owed = usize::from(count).min(disabled);
                    if owed > 0 {
                        self.pending = PendingDecision::ChooseRestores {
                            decider: unit,
                            target,
                            count: u16::try_from(owed).unwrap_or(u16::MAX),
                        };
                    }
                }
                FrozenEffect::Reveal { tier } => {
                    let Some((subject, _)) = target_unit else {
                        continue;
                    };
                    let Some(lattice) = self
                        .units
                        .get(&subject)
                        .and_then(|subject| subject.lattice.as_ref())
                    else {
                        continue;
                    };
                    let cells = lattice.spec.cells().map(|(coord, _)| coord).collect();
                    let rounds = self.rules.reveal_rounds_per_tier.saturating_mul(tier);
                    self.reveals.insert(
                        (actor_faction, subject),
                        self.round.saturating_add(rounds).saturating_add(1),
                    );
                    self.events.push(CombatEvent::Revealed {
                        viewer: actor_faction,
                        subject,
                        cells,
                        rounds,
                    });
                }
            }
        }
        self.units
            .get_mut(&unit)
            .and_then(|actor| actor.turn.as_mut())
            .ok_or(CommandRefusal::NoTurn)?
            .acted = true;
        Ok(())
    }

    fn apply_choose_restores(
        &mut self,
        caster: UnitId,
        target: UnitId,
        cells: &[LatticeCoord],
    ) -> Result<(), CommandRefusal> {
        let PendingDecision::ChooseRestores {
            decider,
            target: expected,
            count,
        } = self.pending
        else {
            return Err(CommandRefusal::Restoration {
                reason: RestorationRefusal::NoDecision,
            });
        };
        if decider != caster {
            return Err(CommandRefusal::WrongDecisionUnit { expected: decider });
        }
        if target != expected {
            return Err(CommandRefusal::Restoration {
                reason: RestorationRefusal::WrongTarget { expected },
            });
        }
        let lattice = self
            .units
            .get(&target)
            .and_then(|target| target.lattice.as_ref())
            .ok_or(CommandRefusal::MissingUnitData {
                unit: target,
                data: UnitData::Lattice,
            })?;
        let disabled = lattice
            .spec
            .cells()
            .filter(|(coord, _)| lattice.state.is_disabled(*coord))
            .count();
        let owed = usize::from(count).min(disabled);
        if cells.len() != owed {
            return Err(CommandRefusal::Restoration {
                reason: RestorationRefusal::WrongCount {
                    expected: u16::try_from(owed).unwrap_or(u16::MAX),
                    actual: u16::try_from(cells.len()).unwrap_or(u16::MAX),
                },
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
            if !lattice.state.is_disabled(cell) {
                return Err(CommandRefusal::Restoration {
                    reason: RestorationRefusal::CellNotDisabled { cell },
                });
            }
        }
        let actor = self
            .units
            .get_mut(&target)
            .ok_or(CommandRefusal::UnknownUnit)?;
        let lattice = actor
            .lattice
            .as_mut()
            .ok_or(CommandRefusal::MissingUnitData {
                unit: target,
                data: UnitData::Lattice,
            })?;
        restore(&mut lattice.state, cells);
        self.pending = PendingDecision::None;
        self.events.push(CombatEvent::HexesRestored {
            caster,
            target,
            cells: cells.to_vec(),
        });
        if !cells.is_empty() && actor.downed {
            actor.downed = false;
            let reenters_round = self.round.saturating_add(1);
            self.pending_revivals.insert(target, reenters_round);
            self.events.push(CombatEvent::Revived {
                unit: target,
                reenters_round,
            });
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
        if self.pending.is_open() || self.resolution_held {
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
                self.insert_due_revivals();
                self.reveals.retain(|_, expiry| *expiry > self.round);
                self.expire_effects();
            }
        }
        self.grant_current_turn();
        self.events.push(CombatEvent::TurnAdvanced {
            unit,
            next: self.current(),
            round: self.round,
        });
    }

    fn insert_due_revivals(&mut self) {
        let due = self
            .pending_revivals
            .iter()
            .filter_map(|(&unit, &round)| (round <= self.round).then_some(unit))
            .collect::<Vec<_>>();
        for unit in &due {
            self.pending_revivals.remove(unit);
            let active = self.units.get(unit).is_some_and(|actor| !actor.downed);
            if active && !self.order.contains(unit) {
                self.order.push(*unit);
            }
        }
        if !due.is_empty() {
            self.order.sort_by_key(|unit| {
                let initiative = self.units.get(unit).map_or(0, |actor| actor.initiative);
                (Reverse(initiative), *unit)
            });
            self.current = self
                .current()
                .and_then(|unit| self.order.iter().position(|candidate| *candidate == unit))
                .unwrap_or_default();
        }
    }

    fn remove_from_order(&mut self, unit: UnitId) {
        let Some(index) = self.order.iter().position(|candidate| *candidate == unit) else {
            return;
        };
        let held_the_turn = self.current() == Some(unit);
        self.order.remove(index);
        if self.order.is_empty() {
            self.current = 0;
        } else if index < self.current {
            self.current = self.current.saturating_sub(1);
        } else if self.current >= self.order.len() {
            self.current = 0;
        }
        if held_the_turn {
            // Removing the current actor slides the index onto its successor. That
            // successor did not pass through `advance_if_finished`, so it has no turn
            // until this explicit handoff. The ECS projection follows the same rule.
            self.grant_current_turn();
        }
    }

    fn detect_outcome(&mut self) {
        if self.resolution_held {
            return;
        }
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
            if self.outcome != Some(outcome) {
                self.outcome = Some(outcome);
                self.events.push(CombatEvent::EncounterResolved { outcome });
            }
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
        if self.pending.is_open() {
            return;
        }

        let source = self
            .effects
            .iter()
            .find(|(_, effect)| {
                effect.target == current
                    && effect.payload == EffectPayload::Burn
                    && !matches!(effect.end, EffectEnd::AfterTurns(turns) if effect.ticks >= turns)
            })
            .map(|(_, effect)| effect.source);
        let mut due = 0_u16;
        for effect in self.effects.values_mut() {
            if effect.target != current || effect.payload != EffectPayload::Burn {
                continue;
            }
            if let EffectEnd::AfterTurns(turns) = effect.end {
                if effect.ticks >= turns {
                    continue;
                }
                effect.ticks = effect.ticks.saturating_add(1);
            }
            due = due.saturating_add(1);
        }
        self.expire_effects();
        if let Some(source) = source.filter(|_| due > 0) {
            let answerable = self
                .units
                .get(&current)
                .and_then(|unit| unit.lattice.as_ref())
                .is_some();
            if answerable {
                self.pending = PendingDecision::ChooseDisables {
                    decider: current,
                    count: due,
                    source,
                };
                self.events.push(CombatEvent::BurnTicked {
                    source,
                    target: current,
                    count: due,
                });
                self.events.push(CombatEvent::DecisionOpened {
                    decider: current,
                    source,
                    count: due,
                });
            }
        }
    }

    fn expire_effects(&mut self) {
        let round = self.round;
        let units = &self.units;
        self.effects.retain(|_, effect| match effect.end {
            EffectEnd::AfterRounds(rounds) => round < effect.start.saturating_add(rounds),
            EffectEnd::AfterTurns(turns) => effect.ticks < turns,
            EffectEnd::WithEnchantment(enchantment) => units
                .get(&effect.target)
                .and_then(|unit| unit.lattice.as_ref())
                .is_some_and(|lattice| lattice.state.enchantment(enchantment).is_some()),
        });
    }

    /// Deterministic fingerprint of the complete serializable authority state.
    ///
    /// # Errors
    ///
    /// Returns an error if the authority ever stops satisfying its serializable
    /// contract. Fingerprint failure is evidence failure and must not collapse to a
    /// plausible sentinel value.
    pub fn fingerprint(&self) -> Result<u64, String> {
        fingerprint(b"hex-combat-authority-v1", self)
    }
}

/// Who supplies commands in a deterministic case.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ControllerInput {
    /// Exact replayable commands, consumed in their recorded order.
    Scripted {
        /// Seat claim retained with every emitted command.
        seat: PlayerSeat,
        /// Exact command payloads, including defender choices.
        commands: Vec<GameCommand>,
    },
    /// Stable non-random reference policy used for unattended comparisons.
    Baseline {
        /// Seat claim retained with every emitted command.
        seat: PlayerSeat,
    },
}

macro_rules! positive_bound {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            /// Creates a non-zero bound.
            pub fn new(value: u32) -> Result<Self, String> {
                (value > 0).then_some(Self(value)).ok_or_else(|| {
                    concat!(stringify!($name), " must be greater than zero").to_owned()
                })
            }

            /// Returns the validated scalar value.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }
        }
    };
}

positive_bound!(CommandBound, "Maximum accepted commands in one run.");
positive_bound!(TurnBound, "Maximum completed turns in one run.");
positive_bound!(
    NoProgressBound,
    "Maximum consecutive turns without movement or an action."
);

/// Independent typed bounds for one deterministic run.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunBounds {
    /// Absolute command ceiling.
    pub commands: CommandBound,
    /// Absolute completed-turn ceiling.
    pub turns: TurnBound,
    /// Consecutive idle-turn ceiling.
    pub no_progress_turns: NoProgressBound,
}

impl RunBounds {
    /// Creates validated run bounds.
    pub fn new(commands: u32, turns: u32, no_progress_turns: u32) -> Result<Self, String> {
        Ok(Self {
            commands: CommandBound::new(commands)?,
            turns: TurnBound::new(turns)?,
            no_progress_turns: NoProgressBound::new(no_progress_turns)?,
        })
    }

    fn validate(self) -> Result<(), String> {
        if self.commands.get() == 0 || self.turns.get() == 0 || self.no_progress_turns.get() == 0 {
            return Err("serialized run bounds must all be greater than zero".to_owned());
        }
        Ok(())
    }
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
    /// Frozen active-combat content.
    #[serde(default)]
    pub content: FrozenCombatContent,
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
        self.bounds.validate()?;
        let mut state = CombatState::start_with_content(
            self.rules.clone(),
            self.arena.clone(),
            self.elements.clone(),
            self.content.clone(),
            self.units.clone(),
        )?;
        let mut cursors = BTreeMap::<UnitId, usize>::new();
        let termination = loop {
            if let Some(outcome) = state.outcome {
                break CombatTermination::Outcome(outcome);
            }
            if state.metrics.successful_commands >= self.bounds.commands.get() {
                break CombatTermination::CommandBoundReached {
                    commands: state.metrics.successful_commands,
                };
            }
            if state.metrics.turns >= self.bounds.turns.get() {
                break CombatTermination::TurnBoundReached {
                    completed_turns: state.metrics.turns,
                };
            }
            if state.metrics.no_progress_current >= self.bounds.no_progress_turns.get() {
                break CombatTermination::NoProgressBoundReached {
                    completed_turns: state.metrics.turns,
                    no_progress_streak: state.metrics.no_progress_current,
                };
            }
            let owner = match state.pending {
                PendingDecision::ChooseDisables { decider, .. }
                | PendingDecision::ChooseRestores { decider, .. } => decider,
                PendingDecision::None => state.current().ok_or_else(|| {
                    format!("case {} has no current unit or terminal outcome", self.name)
                })?,
            };
            let controller = self.controllers.get(&owner).ok_or_else(|| {
                format!("case {} has no controller for unit {owner:?}", self.name)
            })?;
            let (seat, command) = match controller {
                ControllerInput::Scripted { seat, commands } => {
                    let cursor = cursors.entry(owner).or_default();
                    let command = commands.get(*cursor).cloned().ok_or_else(|| {
                        format!(
                            "case {} exhausted the exact script for unit {owner:?} at command {}",
                            self.name, *cursor
                        )
                    })?;
                    *cursor = cursor.saturating_add(1);
                    (*seat, command)
                }
                ControllerInput::Baseline { seat } => (*seat, baseline_command(&state, owner)?),
            };
            if command.unit() != owner {
                return Err(format!(
                    "case {} controller for {owner:?} emitted a command for {:?}",
                    self.name,
                    command.unit()
                ));
            }
            let issued = IssuedCommand { seat, command };
            let moved = matches!(issued.command, GameCommand::MoveAlong { .. });
            if let Err(refusal) = state.apply(issued) {
                return Err(format!(
                    "case {} controller command was refused: {refusal:?}",
                    self.name
                ));
            }
            if moved {
                state.complete_movement(owner).map_err(|error| {
                    format!(
                        "case {} could not settle pure movement: {error:?}",
                        self.name
                    )
                })?;
            }
        };
        CombatRunSnapshot::from_state(self.name.clone(), state, termination)
    }
}

/// Typed reason a bounded run stopped.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatTermination {
    /// Combat reached a retained-world outcome.
    Outcome(EncounterOutcome),
    /// The consecutive idle-turn bound was reached.
    NoProgressBoundReached {
        /// Authoritative turns completed.
        completed_turns: u32,
        /// Current consecutive no-progress turns.
        no_progress_streak: u32,
    },
    /// The absolute accepted-command bound was reached.
    CommandBoundReached {
        /// Accepted commands.
        commands: u32,
    },
    /// The absolute completed-turn bound was reached.
    TurnBoundReached {
        /// Authoritative turns completed.
        completed_turns: u32,
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
    fn from_state(
        case: String,
        state: CombatState,
        termination: CombatTermination,
    ) -> Result<Self, String> {
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
        Ok(Self {
            case,
            summary: state.metrics.clone(),
            state_fingerprint: state.fingerprint()?,
            command_fingerprint: fingerprint(b"hex-combat-commands-v1", &state.commands)?,
            transcript_event_count: state.events.len(),
            transcript_fingerprint: fingerprint(b"hex-combat-transcript-v1", &state.events)?,
            termination,
            turn,
            lattices,
            positions,
            state,
        })
    }
}

fn baseline_command(state: &CombatState, owner: UnitId) -> Result<GameCommand, String> {
    match state.pending {
        PendingDecision::ChooseDisables { decider, count, .. } => {
            let lattice = state
                .units
                .get(&decider)
                .and_then(|unit| unit.lattice.as_ref())
                .ok_or_else(|| format!("baseline decision unit {decider:?} has no lattice"))?;
            let mut cells = lattice
                .spec
                .cells()
                .filter(|(coord, _)| !lattice.state.is_disabled(*coord))
                .map(|(coord, kind)| (cell_priority(kind), coord))
                .collect::<Vec<_>>();
            cells.sort_unstable();
            let owed = usize::from(count).min(cells.len());
            return Ok(GameCommand::ChooseDisables {
                unit: owner,
                cells: cells
                    .into_iter()
                    .take(owed)
                    .map(|(_, coord)| coord)
                    .collect(),
            });
        }
        PendingDecision::ChooseRestores {
            decider,
            target,
            count,
        } => {
            let lattice = state
                .units
                .get(&target)
                .and_then(|unit| unit.lattice.as_ref())
                .ok_or_else(|| format!("baseline restoration target {target:?} has no lattice"))?;
            let mut cells = lattice
                .spec
                .cells()
                .filter(|(coord, _)| lattice.state.is_disabled(*coord))
                .map(|(coord, kind)| (Reverse(cell_priority(kind)), coord))
                .collect::<Vec<_>>();
            cells.sort_unstable();
            let owed = usize::from(count).min(cells.len());
            return Ok(GameCommand::ChooseRestores {
                unit: decider,
                target,
                cells: cells
                    .into_iter()
                    .take(owed)
                    .map(|(_, coord)| coord)
                    .collect(),
            });
        }
        PendingDecision::None => {}
    }

    let actor = state
        .units
        .get(&owner)
        .ok_or_else(|| format!("baseline owns unknown unit {owner:?}"))?;
    let turn = actor
        .turn
        .ok_or_else(|| format!("baseline unit {owner:?} has no turn"))?;
    if turn.acted {
        return Ok(GameCommand::EndTurn { unit: owner });
    }

    if let Some(target) = state.units.values().find(|target| {
        !target.downed
            && actor.faction.is_hostile_to(target.faction)
            && state
                .arena
                .links_for(owner)
                .contains(&(actor.position, target.position))
            && state
                .arena
                .links_for(owner)
                .contains(&(target.position, actor.position))
    }) {
        return Ok(GameCommand::Strike {
            unit: owner,
            target: target.id,
        });
    }

    if let Some(lattice) = actor.lattice.as_ref() {
        for spell in state.content.spells() {
            let Some(cell) = spell_cell(&lattice.spec, &lattice.state, spell.id) else {
                continue;
            };
            if castable(&lattice.spec, &lattice.state, cell, &state.content).is_err() {
                continue;
            }
            let target = match spell.targeting {
                FrozenTargeting::SelfOnly => Some(actor.position),
                FrozenTargeting::ExactSurface { range } => state
                    .units
                    .values()
                    .filter(|candidate| !candidate.downed)
                    .filter(|candidate| {
                        let restoration = spell
                            .effects
                            .iter()
                            .any(|effect| matches!(effect, FrozenEffect::RestoreHexes { .. }));
                        if restoration {
                            candidate.faction == actor.faction
                                && candidate.lattice.as_ref().is_some_and(|lattice| {
                                    lattice
                                        .spec
                                        .cells()
                                        .any(|(coord, _)| lattice.state.is_disabled(coord))
                                })
                        } else {
                            actor.faction.is_hostile_to(candidate.faction)
                        }
                    })
                    .filter(|candidate| state.arena.observes(actor.faction, candidate.position))
                    .filter(|candidate| {
                        let high_ground = actor
                            .position
                            .level
                            .saturating_sub(candidate.position.level)
                            .max(0);
                        let bonus = u32::try_from(high_ground).unwrap_or(u32::MAX)
                            / state.rules.levels_per_bonus_range;
                        actor.position.coord.distance(candidate.position.coord)
                            <= range.saturating_add(bonus)
                    })
                    .map(|candidate| candidate.position)
                    .next(),
            };
            if let Some(target) = target {
                return Ok(GameCommand::Cast {
                    unit: owner,
                    spell: spell.name.clone(),
                    target,
                    facing: None,
                    mana: None,
                });
            }
        }
    }

    let can_restore_mana = actor.lattice.as_ref().is_some_and(|lattice| {
        lattice.spec.cells().any(|(coord, kind)| match kind {
            CellKind::Gem { element } => {
                state.elements.name(element).is_some()
                    && lattice.state.mana(coord) < lattice.stats.capacity(element)
            }
            CellKind::Blank | CellKind::Fusion { .. } | CellKind::Spell { .. } => false,
        })
    });
    if can_restore_mana {
        return Ok(GameCommand::Channel { unit: owner });
    }

    if turn.movement_left > 0 {
        let occupied = state
            .units
            .values()
            .map(|unit| unit.position)
            .collect::<BTreeSet<_>>();
        let hostiles = state
            .units
            .values()
            .filter(|unit| !unit.downed && actor.faction.is_hostile_to(unit.faction))
            .map(|unit| unit.position)
            .collect::<Vec<_>>();
        let distance = |position: TilePos| {
            hostiles
                .iter()
                .map(|target| position.coord.distance(target.coord))
                .min()
                .unwrap_or(u32::MAX)
        };
        let current_distance = distance(actor.position);
        let next = state
            .arena
            .links_for(owner)
            .iter()
            .filter_map(|(from, to)| (*from == actor.position).then_some(*to))
            .filter(|position| !occupied.contains(position))
            .filter(|position| distance(*position) < current_distance)
            .min_by_key(|position| (distance(*position), *position));
        if let Some(next) = next {
            return Ok(GameCommand::MoveAlong {
                unit: owner,
                path: vec![actor.position, next],
            });
        }
    }

    Ok(GameCommand::EndTurn { unit: owner })
}

fn cell_priority(kind: CellKind) -> u8 {
    match kind {
        CellKind::Blank => 0,
        CellKind::Gem { .. } => 1,
        CellKind::Fusion { .. } => 2,
        CellKind::Spell { .. } => 3,
    }
}

fn fingerprint(domain: &[u8], value: &impl Serialize) -> Result<u64, String> {
    let mut bytes = domain.to_vec();
    let encoded = ron::to_string(value).map_err(|error| {
        format!(
            "{} fingerprint serialization failed: {error}",
            String::from_utf8_lossy(domain)
        )
    })?;
    bytes.extend_from_slice(encoded.as_bytes());
    Ok(xxh3_64(&bytes))
}

fn spell_cell(
    spec: &LatticeSpec,
    state: &LatticeState,
    spell: hex_core::SpellId,
) -> Option<LatticeCoord> {
    let mut fallback = None;
    for (coord, kind) in spec.cells() {
        if !matches!(kind, CellKind::Spell { spell: found } if found == spell) {
            continue;
        }
        if !state.is_disabled(coord) {
            return Some(coord);
        }
        fallback = fallback.or(Some(coord));
    }
    fallback
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

    #[test]
    fn movement_is_busy_until_the_domain_route_reaches_its_bound() {
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
                    position(4, 0),
                    10,
                ),
            ],
            4,
        );
        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::MoveAlong {
                unit: UnitId(0),
                path: vec![position(0, 0), position(1, 0)],
            },
        })
        .expect("the exact route is legal");

        let actor = sim.units.get(&UnitId(0)).expect("actor remains");
        assert_eq!(actor.position, position(0, 0));
        assert!(actor.busy);
        assert_eq!(
            actor.motion.as_ref().map(|motion| motion.path.as_slice()),
            Some([position(0, 0), position(1, 0)].as_slice())
        );

        sim.complete_movement(UnitId(0))
            .expect("the committed bound settles");
        let actor = sim.units.get(&UnitId(0)).expect("actor remains");
        assert_eq!(actor.position, position(1, 0));
        assert!(!actor.busy);
        assert!(actor.motion.is_none());
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

    fn spell_content(name: &str, effect: FrozenEffect) -> FrozenCombatContent {
        FrozenCombatContent::new(
            [crate::FrozenSpell {
                id: hex_core::SpellId(0),
                name: name.to_owned(),
                requirements: vec![crate::FrozenRequirement {
                    element: ElementId(0),
                    mana: 1,
                }],
                casting: crate::FrozenCasting::Evocation,
                targeting: FrozenTargeting::ExactSurface { range: 5 },
                effects: vec![effect],
            }],
            [],
        )
        .expect("fixture frozen content")
    }

    fn caster_lattice() -> (LatticeSpec, LatticeState, LatticeStats) {
        let spell = LatticeCoord::ORIGIN;
        let [gem, ..] = spell.neighbors();
        let spec = LatticeSpec::default()
            .with(
                spell,
                CellKind::Spell {
                    spell: hex_core::SpellId(0),
                },
            )
            .with(
                gem,
                CellKind::Gem {
                    element: ElementId(0),
                },
            );
        let stats = LatticeStats::new(
            BTreeMap::from([(ElementId(0), 4)]),
            BTreeMap::from([(ElementId(0), 1)]),
        );
        let state = LatticeState::new(&spec, &stats);
        (spec, state, stats)
    }

    fn blank_lattice(count: usize, disabled: bool) -> (LatticeSpec, LatticeState, LatticeStats) {
        let coords = std::iter::once(LatticeCoord::ORIGIN)
            .chain(LatticeCoord::ORIGIN.neighbors())
            .take(count)
            .collect::<Vec<_>>();
        let spec = coords.iter().fold(LatticeSpec::default(), |spec, &coord| {
            spec.with(coord, CellKind::Blank)
        });
        let stats = LatticeStats::default();
        let mut state = LatticeState::new(&spec, &stats);
        if disabled {
            apply_disables(&mut state, &coords);
        }
        (spec, state, stats)
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
    fn baseline_policy_casts_frozen_content_and_channels_depleted_mana() {
        let (caster_spec, caster_state, caster_stats) = caster_lattice();
        let (target_spec, target_state, target_stats) = blank_lattice(1, false);
        let casting = CombatState::start_with_content(
            rules(4),
            corridor(5),
            ElementNames::new(BTreeMap::from([(ElementId(0), "Fire".to_owned())])),
            spell_content("Spark", FrozenEffect::DisableHexes { count: 1 }),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                )
                .with_lattice(caster_spec, caster_state, caster_stats),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(4, 0),
                    10,
                )
                .with_lattice(target_spec, target_state, target_stats),
            ],
        )
        .expect("casting fixture");
        assert_eq!(
            baseline_command(&casting, UnitId(0)),
            Ok(GameCommand::Cast {
                unit: UnitId(0),
                spell: "Spark".to_owned(),
                target: position(4, 0),
                facing: None,
                mana: None,
            })
        );

        let (fire, spec, state, stats) = depleted_lattice();
        let channeling = CombatState::start(
            rules(4),
            corridor(5),
            ElementNames::new(BTreeMap::from([(fire, "Fire".to_owned())])),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                )
                .with_lattice(spec, state, stats),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(4, 0),
                    10,
                ),
            ],
        )
        .expect("channel fixture");
        assert_eq!(
            baseline_command(&channeling, UnitId(0)),
            Ok(GameCommand::Channel { unit: UnitId(0) })
        );
    }

    #[test]
    fn pure_cast_burn_ticks_at_target_turn_and_opens_exact_choice() {
        let (caster_spec, caster_state, caster_stats) = caster_lattice();
        let (target_spec, target_state, target_stats) = blank_lattice(3, false);
        let mut sim = CombatState::start_with_content(
            rules(4),
            corridor(3),
            ElementNames::new(BTreeMap::from([(ElementId(0), "Fire".to_owned())])),
            spell_content("Cinder", FrozenEffect::Burn { turns: 1 }),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                )
                .with_lattice(caster_spec, caster_state, caster_stats),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(1, 0),
                    10,
                )
                .with_lattice(target_spec, target_state, target_stats),
            ],
        )
        .expect("fixture state");
        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::Cast {
                unit: UnitId(0),
                spell: "Cinder".to_owned(),
                target: position(1, 0),
                facing: None,
                mana: None,
            },
        })
        .expect("pure cast");
        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::EndTurn { unit: UnitId(0) },
        })
        .expect("finish caster turn");
        assert!(matches!(
            sim.pending,
            PendingDecision::ChooseDisables {
                decider: UnitId(1),
                count: 1,
                source: UnitId(0),
            }
        ));
        assert!(
            sim.effects.is_empty(),
            "one-turn Burn expires after its tick"
        );
        assert!(sim.events.iter().any(|event| matches!(
            event,
            CombatEvent::BurnTicked {
                source: UnitId(0),
                target: UnitId(1),
                count: 1,
            }
        )));
    }

    #[test]
    fn pure_restoration_revives_for_the_next_round() {
        let (caster_spec, caster_state, caster_stats) = caster_lattice();
        let (ally_spec, ally_state, ally_stats) = blank_lattice(1, true);
        let mut ally = CombatUnit::new(
            UnitId(1),
            PlayerSeat(0),
            Faction::Player,
            position(1, 0),
            15,
        )
        .with_lattice(ally_spec, ally_state, ally_stats);
        ally.downed = true;
        let mut sim = CombatState::start_with_content(
            rules(4),
            corridor(4),
            ElementNames::new(BTreeMap::from([(ElementId(0), "Life".to_owned())])),
            spell_content("Renew", FrozenEffect::RestoreHexes { count: 1 }),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                )
                .with_lattice(caster_spec, caster_state, caster_stats),
                ally,
                CombatUnit::new(
                    UnitId(2),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(3, 0),
                    10,
                ),
            ],
        )
        .expect("fixture state");
        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::Cast {
                unit: UnitId(0),
                spell: "Renew".to_owned(),
                target: position(1, 0),
                facing: None,
                mana: None,
            },
        })
        .expect("restoration cast");
        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::ChooseRestores {
                unit: UnitId(0),
                target: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN],
            },
        })
        .expect("exact restoration");
        assert!(
            !sim.units
                .get(&UnitId(1))
                .expect("restored unit remains in the simulation")
                .downed
        );
        assert_eq!(sim.pending_revivals.get(&UnitId(1)), Some(&1));
        assert!(sim.events.iter().any(|event| matches!(
            event,
            CombatEvent::Revived {
                unit: UnitId(1),
                reenters_round: 1,
            }
        )));
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
            content: FrozenCombatContent::default(),
            controllers: units
                .iter()
                .map(|unit| {
                    (
                        unit.id,
                        ControllerInput::Scripted {
                            seat: unit.seat,
                            commands: (0..4)
                                .map(|_| GameCommand::EndTurn { unit: unit.id })
                                .collect(),
                        },
                    )
                })
                .collect(),
            units,
            bounds: RunBounds::new(100, 100, 8).expect("fixture bounds"),
        };
        let first = case.run().expect("first run");
        let second = case.run().expect("second run");
        assert_eq!(first, second);
        assert_eq!(first.summary.turns, 8);
        assert_eq!(first.summary.no_progress_current, 8);
        assert!(matches!(
            first.termination,
            CombatTermination::NoProgressBoundReached {
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
    fn invalid_adapter_projection_is_transactional() {
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
                    position(4, 0),
                    10,
                ),
            ],
            3,
        );
        let before = sim.clone();
        let current_turn = sim.units.get(&UnitId(0)).and_then(|unit| unit.turn);
        let invalid = sim.adopt_projection(
            sim.order.clone(),
            sim.current(),
            sim.round,
            sim.pending.clone(),
            BTreeMap::new(),
            [
                CombatUnitProjection {
                    id: UnitId(0),
                    position: position(1, 0),
                    turn: current_turn,
                    busy: false,
                    downed: false,
                    lattice: None,
                },
                CombatUnitProjection {
                    id: UnitId(1),
                    position: position(3, 0),
                    turn: None,
                    busy: false,
                    downed: false,
                    lattice: Some(LatticeState::default()),
                },
            ],
        );
        assert!(invalid.is_err());
        assert_eq!(
            sim, before,
            "a late invalid unit must not leave earlier adapter mutations behind"
        );
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

    #[test]
    fn external_resolution_holds_turn_and_outcome_between_area_answers() {
        let cell = LatticeCoord::ORIGIN;
        let spec = LatticeSpec::default().with(cell, CellKind::Blank);
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
                    position(1, 0),
                    10,
                )
                .with_lattice(
                    spec.clone(),
                    LatticeState::new(&spec, &LatticeStats::default()),
                    LatticeStats::default(),
                ),
            ],
            4,
        );
        let turn = sim
            .units
            .get_mut(&UnitId(0))
            .and_then(|unit| unit.turn.as_mut())
            .expect("the caster owns the opening turn");
        turn.acted = true;
        turn.movement_left = 0;
        sim.pending = PendingDecision::ChooseDisables {
            decider: UnitId(1),
            count: 1,
            source: UnitId(0),
        };
        sim.begin_external_resolution()
            .expect("the area transaction acquires one hold");

        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::ChooseDisables {
                unit: UnitId(1),
                cells: vec![cell],
            },
        })
        .expect("the held defender answer reduces normally");

        assert_eq!(sim.pending, PendingDecision::None);
        assert!(sim.external_resolution_is_held());
        assert_eq!(sim.outcome, None, "the held answer cannot settle victory");
        assert_eq!(sim.metrics.turns, 0, "the held answer cannot pass the turn");
        assert_eq!(sim.current(), Some(UnitId(0)));
        assert_eq!(
            sim.apply(IssuedCommand {
                seat: PlayerSeat(0),
                command: GameCommand::EndTurn { unit: UnitId(0) },
            }),
            Err(CommandRefusal::Busy),
            "ordinary commands stay blocked between queued obligations"
        );

        sim.finish_external_resolution()
            .expect("the complete transaction releases once");
        assert!(!sim.external_resolution_is_held());
        assert_eq!(sim.outcome, Some(EncounterOutcome::Victory));
        assert_eq!(
            sim.metrics.turns, 1,
            "release resumes the finished turn once"
        );
        assert!(sim.finish_external_resolution().is_err());
    }

    #[test]
    fn downing_the_current_decider_grants_its_successor_a_turn() {
        let cell = LatticeCoord::ORIGIN;
        let spec = LatticeSpec::default().with(cell, CellKind::Blank);
        let mut sim = CombatState::start_with_session(
            rules(4),
            corridor(3),
            ElementNames::default(),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    20,
                )
                .with_lattice(
                    spec.clone(),
                    LatticeState::new(&spec, &LatticeStats::default()),
                    LatticeStats::default(),
                ),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(1, 0),
                    10,
                ),
                CombatUnit::new(UnitId(2), PlayerSeat(0), Faction::Player, position(2, 0), 5),
            ],
            PendingDecision::ChooseDisables {
                decider: UnitId(0),
                count: 1,
                source: UnitId(1),
            },
            BTreeMap::new(),
        )
        .expect("fixture state");

        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::ChooseDisables {
                unit: UnitId(0),
                cells: vec![cell],
            },
        })
        .expect("the pending decision should resolve");

        assert!(sim.units.get(&UnitId(0)).is_some_and(|unit| unit.downed));
        assert_eq!(sim.current(), Some(UnitId(1)));
        assert_eq!(
            sim.units.get(&UnitId(1)).and_then(|unit| unit.turn),
            Some(Turn {
                movement_left: 4,
                acted: false,
            }),
            "removing the turn holder must not strand the initiative order"
        );
    }

    #[test]
    fn striking_a_lattice_less_unit_spends_the_action_without_opening_a_decision() {
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
                    position(1, 0),
                    10,
                ),
            ],
            4,
        );

        sim.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::Strike {
                unit: UnitId(0),
                target: UnitId(1),
            },
        })
        .expect("a lattice-less target is still a valid melee target");

        assert_eq!(sim.pending, PendingDecision::None);
        assert!(sim
            .units
            .get(&UnitId(0))
            .and_then(|unit| unit.turn)
            .is_some_and(|turn| turn.acted));
        assert_eq!(
            sim.events
                .iter()
                .filter(|event| matches!(event, CombatEvent::Strike { .. }))
                .count(),
            1
        );
        assert!(!sim.events.iter().any(|event| matches!(
            event,
            CombatEvent::DecisionOpened { .. } | CombatEvent::DamagePrevented { .. }
        )));
    }
}
