//! Pure contracts for deterministic, replaceable enemy decision algorithms.
//!
//! This crate knows what an algorithm may observe and what it may select. It does
//! not know combat legality and cannot mutate the simulation. `hex_combat` owns both:
//! it builds a canonically ordered [`LegalActionSet`] or compact [`CellChoiceSet`],
//! invokes a registered [`AiAlgorithm`], validates the returned [`AiSelection`],
//! and sends the matched [`GameCommand`] through the ordinary command applier.

use bevy_ecs::{prelude::Component, reflect::ReflectComponent};
use bevy_reflect::Reflect;
use hex_core::{
    EffectPayload, GameCommand, KnowledgeState, LatticeCoord, PlayerSeat, TilePos, UnitId,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Stable content identity for one registered decision implementation.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AiAlgorithmId(pub String);

/// Stable content identity for a bundle of algorithm selection and tuning.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AiProfileId(pub String);

/// Optional identity shared by coordinated enemy controllers.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AiGroupId(pub String);

/// Data-authored selection of one registered algorithm.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiProfile {
    /// Stable profile identity referenced by archetypes and encounters.
    pub id: AiProfileId,
    /// Registered implementation this profile dispatches to.
    pub algorithm: AiAlgorithmId,
}

/// The profile and optional coordination group attached to an AI-controlled unit.
#[derive(Component, Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[reflect(Component)]
pub struct AiController {
    /// Content-selected behavior profile.
    pub profile: AiProfileId,
    /// Optional group whose deterministic private state may be shared by the algorithm.
    #[serde(default)]
    pub group: Option<AiGroupId>,
}

/// Which decision point an algorithm is answering.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiDecisionKind {
    /// Choose the acting unit's next normal turn command.
    TurnAction,
    /// Choose exact cells for incoming lattice damage.
    ChooseDisables,
    /// Choose exact disabled cells for a restoration effect.
    ChooseRestores,
}

/// Stable, engine-independent lattice cell category.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AiCellKind {
    /// An uninscribed expendable cell.
    Blank,
    /// A mana-holding gem.
    Gem,
    /// A fusion output.
    Fusion,
    /// An inscribed spell.
    Spell,
}

/// One authorized lattice cell in an observation.
///
/// Unknown hostile facts remain [`None`]. Stable content names are used for kinds
/// rather than session-local IDs.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiLatticeCell {
    /// Stable cell position.
    pub coord: LatticeCoord,
    /// Stable authored kind name when authorized.
    pub kind: Option<AiCellKind>,
    /// Whether the cell is disabled when authorized.
    pub disabled: Option<bool>,
    /// Current mana when authorized.
    pub mana: Option<u16>,
}

/// Authorized, policy-neutral facts about one spell available to an allied unit.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiSpellObservation {
    /// Stable authored spell name.
    pub name: String,
    /// Raw direct disables before defenses.
    pub direct_disables: u16,
    /// Whether the shape has one positional subject.
    pub single_target: bool,
    /// Whether this is a self enchantment.
    pub self_enchantment: bool,
    /// Whether that enchantment is already active.
    pub enchantment_active: bool,
}

/// Authorized lattice projection for one unit.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiLatticeObservation {
    /// Capacity is hidden from hostiles until faction knowledge reveals it.
    pub capacity: Option<u16>,
    /// Known cells in stable coordinate order.
    pub cells: Vec<AiLatticeCell>,
}

/// One effect relevant to a decision.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiEffectObservation {
    /// Stable source identity.
    pub source: UnitId,
    /// Stable target identity.
    pub target: UnitId,
    /// Exact domain payload, never presentation text.
    pub payload: EffectPayload,
}

/// Full authorized information for the actor or one ally.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiAlliedUnit {
    /// Stable unit identity.
    pub unit: UnitId,
    /// Exact current surface.
    pub position: TilePos,
    /// Whether the unit is downed.
    pub downed: bool,
    /// Complete allied lattice state.
    pub lattice: AiLatticeObservation,
    /// Castable spell facts in stable name order.
    pub spells: Vec<AiSpellObservation>,
}

/// Observed information for one hostile.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiObservedHostile {
    /// Stable unit identity.
    pub unit: UnitId,
    /// Exact observed surface.
    pub position: TilePos,
    /// Whether the observed hostile is already downed.
    #[serde(default)]
    pub downed: bool,
    /// Hostile lattice facts projected only through faction knowledge.
    pub lattice: AiLatticeObservation,
}

/// One surface the controller is authorized to plan across.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiTraversalObservation {
    /// Exact surface identity.
    pub position: TilePos,
    /// Faction-knowledge state for this surface.
    pub knowledge: KnowledgeState,
    /// Whether authorized traversal knowledge currently admits the actor.
    pub standable: bool,
    /// Exact directed terrain edges from this surface in stable position order.
    pub neighbors: Vec<TilePos>,
}

/// Everything an algorithm is allowed to inspect for one request.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiObservation {
    /// Full acting-unit information.
    pub actor: AiAlliedUnit,
    /// Other allied units in stable unit-id order.
    pub allies: Vec<AiAlliedUnit>,
    /// Currently observed hostiles in stable unit-id order.
    pub hostiles: Vec<AiObservedHostile>,
    /// Current and upcoming actors in authoritative turn order.
    pub turn_order: Vec<UnitId>,
    /// Zero-based combat round.
    pub round: u32,
    /// Effects relevant to the decision in stable domain order.
    pub effects: Vec<AiEffectObservation>,
    /// Authorized traversal projection in stable position order.
    pub traversal: Vec<AiTraversalObservation>,
}

/// Stable fingerprint for one exact, canonically ordered legal-action set.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct LegalActionFingerprint(pub u64);

/// Stable fingerprint for one exact cell-choice request.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct CellChoiceFingerprint(pub u64);

/// Opaque selection key, valid only with the request fingerprint that issued it.
#[derive(
    Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct ActionKey {
    fingerprint: LegalActionFingerprint,
    ordinal: u32,
}

impl ActionKey {
    /// Builds a key for adapters and test algorithms.
    ///
    /// Production algorithms normally return a key borrowed from
    /// [`LegalActionSet::actions`]. Keeping this constructor explicit lets the host
    /// exercise stale and unknown-key handling without exposing either field.
    #[must_use]
    pub const fn from_parts(fingerprint: LegalActionFingerprint, ordinal: u32) -> Self {
        Self {
            fingerprint,
            ordinal,
        }
    }

    /// Fingerprint of the request that issued this key.
    #[must_use]
    pub const fn fingerprint(self) -> LegalActionFingerprint {
        self.fingerprint
    }

    /// Canonical ordinal within that request.
    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }
}

/// One exact command an algorithm may select.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LegalAction {
    /// Request-scoped key returned by algorithms.
    pub key: ActionKey,
    /// Exact replayable command sent through the ordinary applier.
    pub command: GameCommand,
}

/// Canonically ordered legal commands and their shared fingerprint.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LegalActionSet {
    fingerprint: LegalActionFingerprint,
    actions: Vec<LegalAction>,
}

impl LegalActionSet {
    /// Assigns request-scoped keys to commands already sorted by semantic tuples.
    ///
    /// `hex_combat` computes the fingerprint from the same canonical command
    /// encoding. This constructor makes it impossible to pair an action with a key
    /// from a different fingerprint or ordinal.
    #[must_use]
    pub fn from_canonical_commands(
        fingerprint: LegalActionFingerprint,
        commands: Vec<GameCommand>,
    ) -> Self {
        let actions = commands
            .into_iter()
            .enumerate()
            .filter_map(|(ordinal, command)| {
                u32::try_from(ordinal).ok().map(|ordinal| LegalAction {
                    key: ActionKey {
                        fingerprint,
                        ordinal,
                    },
                    command,
                })
            })
            .collect();
        Self {
            fingerprint,
            actions,
        }
    }

    /// Fingerprint binding every key in this set.
    #[must_use]
    pub const fn fingerprint(&self) -> LegalActionFingerprint {
        self.fingerprint
    }

    /// Actions in canonical semantic order.
    #[must_use]
    pub fn actions(&self) -> &[LegalAction] {
        &self.actions
    }

    /// Resolves a selection only when both fingerprint and ordinal match.
    #[must_use]
    pub fn resolve(&self, key: ActionKey) -> Option<&LegalAction> {
        if key.fingerprint != self.fingerprint {
            return None;
        }
        self.actions
            .iter()
            .find(|action| action.key.ordinal == key.ordinal)
    }
}

/// A compact exact-cell decision without materializing every combination.
///
/// The eligible coordinates are canonical and complete. An algorithm chooses exactly
/// [`Self::count`] distinct members, and the host validates that selection before it
/// constructs the ordinary replayable command.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CellChoiceSet {
    fingerprint: CellChoiceFingerprint,
    subject: UnitId,
    count: u16,
    eligible: Vec<LatticeCoord>,
}

impl CellChoiceSet {
    /// Builds a request from its canonical coordinate set.
    ///
    /// Sorting and deduplication here keep adapters from accidentally issuing a
    /// request whose validity depends on source iteration order.
    #[must_use]
    pub fn from_cells(
        fingerprint: CellChoiceFingerprint,
        subject: UnitId,
        count: u16,
        mut eligible: Vec<LatticeCoord>,
    ) -> Self {
        eligible.sort_unstable();
        eligible.dedup();
        Self {
            fingerprint,
            subject,
            count: count.min(u16::try_from(eligible.len()).unwrap_or(u16::MAX)),
            eligible,
        }
    }

    /// Fingerprint binding the quota and complete eligible set.
    #[must_use]
    pub const fn fingerprint(&self) -> CellChoiceFingerprint {
        self.fingerprint
    }

    /// Unit whose lattice supplies the eligible cells.
    #[must_use]
    pub const fn subject(&self) -> UnitId {
        self.subject
    }

    /// Exact number of distinct cells the algorithm must return.
    #[must_use]
    pub const fn count(&self) -> u16 {
        self.count
    }

    /// Eligible cells in canonical coordinate order.
    #[must_use]
    pub fn eligible(&self) -> &[LatticeCoord] {
        &self.eligible
    }

    /// Builds a request-bound selection.
    #[must_use]
    pub fn selection(&self, cells: Vec<LatticeCoord>) -> CellSelection {
        CellSelection {
            fingerprint: self.fingerprint,
            cells,
        }
    }

    /// Validates fingerprint, quota, uniqueness, and membership.
    pub fn validate(&self, selection: &CellSelection) -> Result<(), AiDecisionFailure> {
        if selection.fingerprint != self.fingerprint {
            return Err(AiDecisionFailure::StaleFingerprint);
        }
        let actual = u16::try_from(selection.cells.len()).unwrap_or(u16::MAX);
        if actual != self.count {
            return Err(AiDecisionFailure::WrongCellCount);
        }
        let mut cells = selection.cells.clone();
        cells.sort_unstable();
        if cells
            .windows(2)
            .any(|pair| matches!(pair, [left, right] if left == right))
        {
            return Err(AiDecisionFailure::DuplicateCell);
        }
        if cells
            .iter()
            .any(|cell| self.eligible.binary_search(cell).is_err())
        {
            return Err(AiDecisionFailure::IneligibleCell);
        }
        Ok(())
    }
}

/// Exact cells selected for one compact request.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CellSelection {
    fingerprint: CellChoiceFingerprint,
    /// Exact replayable coordinates chosen by the algorithm.
    pub cells: Vec<LatticeCoord>,
}

impl CellSelection {
    /// Fingerprint of the request that issued this selection.
    #[must_use]
    pub const fn fingerprint(&self) -> CellChoiceFingerprint {
        self.fingerprint
    }
}

/// One selection returned by an algorithm.
///
/// `untagged` preserves the serialized shape of legacy action-key traces while adding
/// the distinct cell-selection object for compact damage and restoration decisions.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum AiSelection {
    /// One opaque normal-turn action key.
    Action(ActionKey),
    /// Exact cells for a compact lattice decision.
    Cells(CellSelection),
}

/// Complete input to one registered algorithm call.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DecisionRequest {
    /// Seat whose controller is making the request.
    pub controller: PlayerSeat,
    /// Optional coordination group.
    pub group: Option<AiGroupId>,
    /// Decision point being answered.
    pub kind: AiDecisionKind,
    /// Authorized domain snapshot.
    pub observation: AiObservation,
    /// The complete legal choice set.
    pub legal_actions: LegalActionSet,
    /// Compact exact-cell domain for damage or restoration decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_choices: Option<CellChoiceSet>,
}

/// Mutable, session-scoped implementation selected by an AI profile.
///
/// Implementations may retain deterministic actor/group state. The runtime clears
/// algorithm instances on gameplay teardown; persistence belongs to Wave 5.
pub trait AiAlgorithm: Send + Sync + 'static {
    /// Selects one request-scoped action or exact cell set without mutating game state.
    fn select(&mut self, request: &DecisionRequest) -> AiSelection;
}

/// Shipped deterministic combat policy.
#[derive(Debug, Default)]
pub struct BaselineAlgorithm;

impl AiAlgorithm for BaselineAlgorithm {
    fn select(&mut self, request: &DecisionRequest) -> AiSelection {
        let actions = request.legal_actions.actions();
        let fallback = actions
            .iter()
            .find(|action| matches!(action.command, GameCommand::EndTurn { .. }))
            .or_else(|| actions.first())
            .map(|action| action.key)
            .unwrap_or_else(|| ActionKey::from_parts(request.legal_actions.fingerprint(), 0));

        match request.kind {
            AiDecisionKind::ChooseDisables => {
                choose_cells(request, false).unwrap_or(AiSelection::Action(fallback))
            }
            AiDecisionKind::ChooseRestores => {
                choose_cells(request, true).unwrap_or(AiSelection::Action(fallback))
            }
            AiDecisionKind::TurnAction => {
                AiSelection::Action(choose_turn_action(request).unwrap_or(fallback))
            }
        }
    }
}

fn choose_cells(request: &DecisionRequest, restoring: bool) -> Option<AiSelection> {
    let choices = request.cell_choices.as_ref()?;
    let lattice_unit = choices.subject();
    let lattice = if request.observation.actor.unit == lattice_unit {
        Some(&request.observation.actor.lattice)
    } else {
        request
            .observation
            .allies
            .iter()
            .find(|ally| ally.unit == lattice_unit)
            .map(|ally| &ally.lattice)
    }?;
    let mut ranked: Vec<(u8, u16, LatticeCoord)> = choices
        .eligible()
        .iter()
        .filter_map(|coord| {
            let cell = lattice.cells.iter().find(|cell| cell.coord == *coord)?;
            let kind = cell.kind?;
            let rank = if restoring {
                match kind {
                    AiCellKind::Spell => 0,
                    AiCellKind::Fusion => 1,
                    AiCellKind::Gem => 2,
                    AiCellKind::Blank => 3,
                }
            } else {
                match kind {
                    AiCellKind::Blank => 0,
                    AiCellKind::Gem => 1,
                    AiCellKind::Fusion => 2,
                    AiCellKind::Spell => 3,
                }
            };
            Some((rank, cell.mana.unwrap_or(0), *coord))
        })
        .collect();
    ranked.sort_unstable();
    let cells = ranked
        .into_iter()
        .take(usize::from(choices.count()))
        .map(|(_, _, coord)| coord)
        .collect();
    Some(AiSelection::Cells(choices.selection(cells)))
}

fn choose_turn_action(request: &DecisionRequest) -> Option<ActionKey> {
    let actions = request.legal_actions.actions();
    let traversal = ReverseTraversal::new(&request.observation.traversal);
    let hostile_distances: Vec<(UnitId, Vec<u32>)> = request
        .observation
        .hostiles
        .iter()
        .filter(|hostile| !hostile.downed)
        .map(|hostile| (hostile.unit, traversal.distances_to_melee(hostile.position)))
        .collect();
    let cast_named_at = |name: &str, target: TilePos| {
        actions.iter().find_map(|action| match &action.command {
            GameCommand::Cast {
                spell,
                target: found,
                ..
            } if spell == name && *found == target => Some(action.key),
            _ => None,
        })
    };

    if let Some(key) = request
        .observation
        .allies
        .iter()
        .filter(|ally| ally.downed)
        .min_by_key(|ally| ally.unit)
        .and_then(|ally| cast_named_at("Renewal", ally.position))
    {
        return Some(key);
    }
    if let Some(key) = request
        .observation
        .hostiles
        .iter()
        .filter(|hostile| hostile.lattice.capacity.is_none())
        .min_by_key(|hostile| hostile.unit)
        .and_then(|hostile| cast_named_at("Scrying Eye", hostile.position))
    {
        return Some(key);
    }

    let spell_facts = &request.observation.actor.spells;
    if let Some(key) = actions
        .iter()
        .filter_map(|action| {
            let GameCommand::Cast { spell, target, .. } = &action.command else {
                return None;
            };
            let facts = spell_facts.iter().find(|facts| {
                facts.name == *spell && facts.single_target && facts.direct_disables > 0
            })?;
            let target_id = request
                .observation
                .hostiles
                .iter()
                .find(|hostile| hostile.position == *target)
                .map(|hostile| hostile.unit)?;
            Some((
                std::cmp::Reverse(facts.direct_disables),
                spell.as_str(),
                target_id,
                action.key,
            ))
        })
        .min_by(|left, right| (left.0, left.1, left.2).cmp(&(right.0, right.1, right.2)))
        .map(|(_, _, _, key)| key)
    {
        return Some(key);
    }

    if let Some(key) = actions
        .iter()
        .filter_map(|action| {
            let GameCommand::Cast { spell, .. } = &action.command else {
                return None;
            };
            spell_facts
                .iter()
                .find(|facts| {
                    facts.name == *spell && facts.self_enchantment && !facts.enchantment_active
                })
                .map(|_| (spell.as_str(), action.key))
        })
        .min_by(|left, right| left.0.cmp(right.0))
        .map(|(_, key)| key)
    {
        return Some(key);
    }

    if let Some(key) = actions
        .iter()
        .filter_map(|action| match action.command {
            GameCommand::Strike { target, .. } => Some((target, action.key)),
            _ => None,
        })
        .min_by_key(|(target, _)| *target)
        .map(|(_, key)| key)
    {
        return Some(key);
    }

    if request.observation.actor.lattice.cells.iter().any(|cell| {
        cell.kind == Some(AiCellKind::Gem) && cell.disabled == Some(false) && cell.mana == Some(0)
    }) {
        if let Some(key) = actions.iter().find_map(|action| {
            matches!(action.command, GameCommand::Channel { .. }).then_some(action.key)
        }) {
            return Some(key);
        }
    }

    actions
        .iter()
        .filter_map(|action| match &action.command {
            GameCommand::MoveAlong { path, .. } => {
                let endpoint = path.last()?;
                let used = u32::try_from(path.len().saturating_sub(1)).unwrap_or(u32::MAX);
                let nearest = hostile_distances
                    .iter()
                    .filter_map(|(unit, distances)| {
                        traversal
                            .distance_at(distances, *endpoint)
                            .map(|remaining| (used.saturating_add(remaining), *unit, remaining))
                    })
                    .min()?;
                Some((
                    nearest.0,
                    nearest.1,
                    nearest.2,
                    std::cmp::Reverse(used),
                    action.key,
                ))
            }
            _ => None,
        })
        .min_by_key(|(total, target, remaining, used, _)| (*total, *target, *remaining, *used))
        .map(|(_, _, _, _, key)| key)
}

/// One canonical reverse traversal index shared by every candidate in a decision.
struct ReverseTraversal<'a> {
    surfaces: Vec<&'a AiTraversalObservation>,
    index_by_position: HashMap<TilePos, usize>,
    predecessors: Vec<Vec<usize>>,
}

impl<'a> ReverseTraversal<'a> {
    fn new(traversal: &'a [AiTraversalObservation]) -> Self {
        let mut surfaces: Vec<_> = traversal.iter().collect();
        surfaces.sort_unstable_by_key(|surface| surface.position);
        surfaces.dedup_by_key(|surface| surface.position);
        let index_by_position: HashMap<_, _> = surfaces
            .iter()
            .enumerate()
            .map(|(index, surface)| (surface.position, index))
            .collect();
        let mut predecessors = vec![Vec::new(); surfaces.len()];
        for (source_index, surface) in surfaces.iter().enumerate() {
            if !surface.standable {
                continue;
            }
            for &neighbor in &surface.neighbors {
                if let Some(&neighbor_index) = index_by_position.get(&neighbor) {
                    if !surfaces
                        .get(neighbor_index)
                        .is_some_and(|candidate| candidate.standable)
                    {
                        continue;
                    }
                    if let Some(incoming) = predecessors.get_mut(neighbor_index) {
                        incoming.push(source_index);
                    }
                }
            }
        }
        for incoming in &mut predecessors {
            incoming.sort_unstable();
            incoming.dedup();
        }
        Self {
            surfaces,
            index_by_position,
            predecessors,
        }
    }

    /// Returns every authorized start's shortest directed distance to melee range.
    fn distances_to_melee(&self, target: TilePos) -> Vec<u32> {
        let mut distances = vec![u32::MAX; self.surfaces.len()];
        let Some(&target_index) = self.index_by_position.get(&target) else {
            return distances;
        };
        let Some(target_surface) = self.surfaces.get(target_index) else {
            return distances;
        };
        let mut goals: Vec<usize> = target_surface
            .neighbors
            .iter()
            .filter_map(|position| self.index_by_position.get(position).copied())
            .filter(|&index| {
                self.surfaces
                    .get(index)
                    .is_some_and(|surface| surface.standable && surface.neighbors.contains(&target))
            })
            .collect();
        goals.sort_unstable();
        goals.dedup();

        let mut frontier = VecDeque::new();
        for goal in goals {
            if let Some(distance) = distances.get_mut(goal) {
                *distance = 0;
                frontier.push_back(goal);
            }
        }
        while let Some(at) = frontier.pop_front() {
            let Some(&cost) = distances.get(at) else {
                continue;
            };
            let Some(predecessors) = self.predecessors.get(at) else {
                continue;
            };
            for &previous in predecessors {
                let Some(previous_distance) = distances.get_mut(previous) else {
                    continue;
                };
                if *previous_distance != u32::MAX {
                    continue;
                }
                *previous_distance = cost.saturating_add(1);
                frontier.push_back(previous);
            }
        }
        distances
    }

    fn distance_at(&self, distances: &[u32], position: TilePos) -> Option<u32> {
        let index = self.index_by_position.get(&position)?;
        distances
            .get(*index)
            .copied()
            .filter(|distance| *distance != u32::MAX)
    }
}

/// Why a requested AI selection did not produce its named command.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiDecisionFailure {
    /// The algorithm returned a key from another request fingerprint.
    StaleFingerprint,
    /// The fingerprint matched but no action had the returned ordinal.
    UnknownAction,
    /// The algorithm returned a normal action for a cell request, or vice versa.
    WrongSelectionKind,
    /// A cell selection did not contain the exact requested quota.
    WrongCellCount,
    /// A cell selection repeated one coordinate.
    DuplicateCell,
    /// A cell selection named a coordinate outside the eligible set.
    IneligibleCell,
}

/// Development trace for one deterministic AI dispatch.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiDecisionTrace {
    /// Monotonic session-local dispatch sequence assigned by the host.
    #[serde(default)]
    pub sequence: u64,
    /// Selected content profile.
    pub profile: AiProfileId,
    /// Registered implementation.
    pub algorithm: AiAlgorithmId,
    /// Stable acting unit.
    pub actor: UnitId,
    /// Optional coordination group.
    pub group: Option<AiGroupId>,
    /// Decision point being answered.
    pub kind: AiDecisionKind,
    /// Exact authorized observation supplied to the algorithm.
    pub observation: AiObservation,
    /// Canonical commands and request-scoped keys offered to the algorithm.
    pub legal_actions: LegalActionSet,
    /// Fingerprint of the offered legal set.
    pub fingerprint: LegalActionFingerprint,
    /// Fingerprint of the compact cell set, when this was a lattice decision.
    #[serde(default)]
    pub cell_fingerprint: Option<CellChoiceFingerprint>,
    /// Returned action or exact cells.
    pub selected: AiSelection,
    /// Command ultimately sent through the applier, including deterministic fallback.
    pub command: Option<GameCommand>,
    /// Failure when the key could not resolve.
    pub failure: Option<AiDecisionFailure>,
}

/// Compact deterministic record retained by long-running combat summaries.
///
/// Full observations and legal domains belong in the short live trace window or the
/// opt-in transcript recorder. This record keeps enough information to audit dispatch,
/// selection, fallback, and replay outcome without retaining a map projection per turn.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiDecisionRecord {
    /// Monotonic session-local dispatch sequence.
    #[serde(default)]
    pub sequence: u64,
    /// Selected content profile.
    pub profile: AiProfileId,
    /// Registered implementation.
    pub algorithm: AiAlgorithmId,
    /// Stable acting unit.
    pub actor: UnitId,
    /// Optional coordination group.
    pub group: Option<AiGroupId>,
    /// Decision point answered.
    pub kind: AiDecisionKind,
    /// Fingerprint of the offered normal-action set.
    pub fingerprint: LegalActionFingerprint,
    /// Fingerprint of the compact cell set, when applicable.
    #[serde(default)]
    pub cell_fingerprint: Option<CellChoiceFingerprint>,
    /// Returned action or exact cells.
    pub selected: AiSelection,
    /// Command sent through the applier, including fallback.
    pub command: Option<GameCommand>,
    /// Failure that caused fallback.
    pub failure: Option<AiDecisionFailure>,
}

impl From<&AiDecisionTrace> for AiDecisionRecord {
    fn from(trace: &AiDecisionTrace) -> Self {
        Self {
            sequence: trace.sequence,
            profile: trace.profile.clone(),
            algorithm: trace.algorithm.clone(),
            actor: trace.actor,
            group: trace.group.clone(),
            kind: trace.kind,
            fingerprint: trace.fingerprint,
            cell_fingerprint: trace.cell_fingerprint,
            selected: trace.selected.clone(),
            command: trace.command.clone(),
            failure: trace.failure,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::{Duration, Instant};

    use hex_core::{HexCoord, KnowledgeState};

    use super::*;

    struct FirstAlgorithm;

    impl AiAlgorithm for FirstAlgorithm {
        fn select(&mut self, request: &DecisionRequest) -> AiSelection {
            AiSelection::Action(request.legal_actions.actions().first().map_or_else(
                || ActionKey::from_parts(request.legal_actions.fingerprint(), 0),
                |action| action.key,
            ))
        }
    }

    struct LastAlgorithm;

    impl AiAlgorithm for LastAlgorithm {
        fn select(&mut self, request: &DecisionRequest) -> AiSelection {
            AiSelection::Action(request.legal_actions.actions().last().map_or_else(
                || ActionKey::from_parts(request.legal_actions.fingerprint(), 0),
                |action| action.key,
            ))
        }
    }

    fn request() -> DecisionRequest {
        let actor = AiAlliedUnit {
            unit: UnitId(1),
            position: TilePos::new(HexCoord::ORIGIN, 1),
            downed: false,
            lattice: AiLatticeObservation {
                capacity: Some(1),
                cells: vec![AiLatticeCell {
                    coord: LatticeCoord::ORIGIN,
                    kind: Some(AiCellKind::Spell),
                    disabled: Some(false),
                    mana: None,
                }],
            },
            spells: Vec::new(),
        };
        DecisionRequest {
            controller: PlayerSeat(0),
            group: Some(AiGroupId("raiders".to_owned())),
            kind: AiDecisionKind::TurnAction,
            observation: AiObservation {
                actor,
                allies: Vec::new(),
                hostiles: Vec::new(),
                turn_order: vec![UnitId(1)],
                round: 0,
                effects: Vec::new(),
                traversal: vec![AiTraversalObservation {
                    position: TilePos::new(HexCoord::ORIGIN, 1),
                    knowledge: KnowledgeState::Observed,
                    standable: true,
                    neighbors: Vec::new(),
                }],
            },
            legal_actions: LegalActionSet::from_canonical_commands(
                LegalActionFingerprint(42),
                vec![GameCommand::EndTurn { unit: UnitId(1) }],
            ),
            cell_choices: None,
        }
    }

    #[test]
    fn decision_request_round_trips() {
        let value = request();
        let encoded = serde_json::to_string(&value).expect("request serializes");
        let decoded: DecisionRequest =
            serde_json::from_str(&encoded).expect("request deserializes");
        assert_eq!(decoded, value);
    }

    #[test]
    fn stale_or_foreign_keys_do_not_resolve() {
        let request = request();
        let action = request
            .legal_actions
            .actions()
            .first()
            .expect("fixture has end turn");
        assert_eq!(
            request.legal_actions.resolve(action.key),
            Some(action),
            "the issued key resolves"
        );
        assert_eq!(
            request.legal_actions.resolve(ActionKey {
                fingerprint: LegalActionFingerprint(41),
                ordinal: 0,
            }),
            None
        );
        assert_eq!(
            request.legal_actions.resolve(ActionKey {
                fingerprint: LegalActionFingerprint(42),
                ordinal: 1,
            }),
            None
        );
    }

    #[test]
    fn replaceable_algorithms_can_choose_different_legal_commands() {
        let mut request = request();
        request.legal_actions = LegalActionSet::from_canonical_commands(
            LegalActionFingerprint(9),
            vec![
                GameCommand::Strike {
                    unit: UnitId(1),
                    target: UnitId(2),
                },
                GameCommand::EndTurn { unit: UnitId(1) },
            ],
        );
        let first = FirstAlgorithm.select(&request);
        let last = LastAlgorithm.select(&request);
        assert_ne!(first, last);
        let AiSelection::Action(first) = first else {
            panic!("first algorithm should return an action");
        };
        let AiSelection::Action(last) = last else {
            panic!("last algorithm should return an action");
        };
        assert!(matches!(
            request
                .legal_actions
                .resolve(first)
                .map(|action| &action.command),
            Some(GameCommand::Strike { .. })
        ));
        assert!(matches!(
            request
                .legal_actions
                .resolve(last)
                .map(|action| &action.command),
            Some(GameCommand::EndTurn { .. })
        ));
    }

    #[test]
    fn baseline_channels_a_depleted_lattice_from_the_canonical_set() {
        let mut request = request();
        request.observation.actor.lattice.cells = vec![AiLatticeCell {
            coord: LatticeCoord::ORIGIN,
            kind: Some(AiCellKind::Gem),
            disabled: Some(false),
            mana: Some(0),
        }];
        request.legal_actions = LegalActionSet::from_canonical_commands(
            LegalActionFingerprint(10),
            vec![
                GameCommand::EndTurn { unit: UnitId(1) },
                GameCommand::Channel { unit: UnitId(1) },
            ],
        );

        let AiSelection::Action(key) = BaselineAlgorithm.select(&request) else {
            panic!("a normal turn should select an action");
        };
        assert!(matches!(
            request
                .legal_actions
                .resolve(key)
                .map(|action| &action.command),
            Some(GameCommand::Channel { unit: UnitId(1) })
        ));
    }

    #[test]
    fn compact_cell_choices_validate_without_materializing_combinations() {
        let eligible = (0..32).map(|x| LatticeCoord::new(x, 0)).collect::<Vec<_>>();
        let choices = CellChoiceSet::from_cells(CellChoiceFingerprint(77), UnitId(4), 8, eligible);
        assert_eq!(choices.eligible().len(), 32);
        assert_eq!(choices.count(), 8);
        let valid = choices.selection(
            choices
                .eligible()
                .iter()
                .copied()
                .take(8)
                .collect::<Vec<_>>(),
        );
        assert_eq!(choices.validate(&valid), Ok(()));

        let duplicate = choices.selection(vec![LatticeCoord::ORIGIN; 8]);
        assert_eq!(
            choices.validate(&duplicate),
            Err(AiDecisionFailure::DuplicateCell)
        );
        let outside = choices.selection(
            (0..7)
                .map(|x| LatticeCoord::new(x, 0))
                .chain([LatticeCoord::new(99, 0)])
                .collect(),
        );
        assert_eq!(
            choices.validate(&outside),
            Err(AiDecisionFailure::IneligibleCell)
        );
    }

    fn benchmark_ally(unit: UnitId, position: TilePos) -> AiAlliedUnit {
        AiAlliedUnit {
            unit,
            position,
            downed: false,
            lattice: AiLatticeObservation {
                capacity: None,
                cells: Vec::new(),
            },
            spells: Vec::new(),
        }
    }

    fn mix_fingerprint(mut fingerprint: u64, bytes: &[u8]) -> u64 {
        for &byte in bytes {
            fingerprint ^= u64::from(byte);
            fingerprint = fingerprint.wrapping_mul(1_099_511_628_211);
        }
        fingerprint
    }

    fn benchmark_request(radius: u32, team_size: usize) -> DecisionRequest {
        let position = |coord| TilePos::new(coord, 1);
        let mut coords = HexCoord::ORIGIN.within_radius(radius);
        coords.sort_unstable();
        let coordinate_set: BTreeSet<HexCoord> = coords.iter().copied().collect();
        let traversal = coords
            .iter()
            .copied()
            .map(|coord| {
                let mut neighbors = coord
                    .neighbors()
                    .into_iter()
                    .filter(|neighbor| coordinate_set.contains(neighbor))
                    .map(position)
                    .collect::<Vec<_>>();
                neighbors.sort_unstable();
                AiTraversalObservation {
                    position: position(coord),
                    knowledge: KnowledgeState::Observed,
                    standable: true,
                    neighbors,
                }
            })
            .collect();

        let allied_offsets = [
            HexCoord::from_axial(-1, 0),
            HexCoord::from_axial(0, -1),
            HexCoord::from_axial(1, -1),
            HexCoord::from_axial(1, 0),
            HexCoord::from_axial(0, 1),
        ];
        let allies = allied_offsets
            .into_iter()
            .take(team_size.saturating_sub(1))
            .enumerate()
            .map(|(index, coord)| {
                benchmark_ally(
                    UnitId(u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1)),
                    position(coord),
                )
            })
            .collect::<Vec<_>>();

        let distance = i32::try_from(radius.saturating_sub(2)).unwrap_or(i32::MAX);
        let hostile_offsets = [
            HexCoord::from_axial(distance, 0),
            HexCoord::from_axial(0, distance),
            HexCoord::from_axial(-distance, distance),
            HexCoord::from_axial(-distance, 0),
            HexCoord::from_axial(0, -distance),
            HexCoord::from_axial(distance, -distance),
        ];
        let hostiles = hostile_offsets
            .into_iter()
            .take(team_size)
            .enumerate()
            .map(|(index, coord)| AiObservedHostile {
                unit: UnitId(u64::try_from(index).unwrap_or(u64::MAX).saturating_add(100)),
                position: position(coord),
                downed: false,
                lattice: AiLatticeObservation {
                    capacity: None,
                    cells: Vec::new(),
                },
            })
            .collect::<Vec<_>>();

        let mut endpoints = HexCoord::ORIGIN.within_radius(4);
        endpoints.sort_unstable();
        let mut commands = endpoints
            .into_iter()
            .filter(|coord| *coord != HexCoord::ORIGIN)
            .map(|endpoint| GameCommand::MoveAlong {
                unit: UnitId(0),
                path: HexCoord::ORIGIN
                    .line_between(endpoint)
                    .into_iter()
                    .map(position)
                    .collect(),
            })
            .collect::<Vec<_>>();
        commands.push(GameCommand::EndTurn { unit: UnitId(0) });

        let mut fingerprint = 14_695_981_039_346_656_037;
        fingerprint = mix_fingerprint(fingerprint, &radius.to_le_bytes());
        fingerprint = mix_fingerprint(
            fingerprint,
            &u64::try_from(team_size).unwrap_or(u64::MAX).to_le_bytes(),
        );
        for command in &commands {
            if let GameCommand::MoveAlong { path, .. } = command {
                for step in path {
                    fingerprint = mix_fingerprint(fingerprint, &step.coord.x().to_le_bytes());
                    fingerprint = mix_fingerprint(fingerprint, &step.coord.y().to_le_bytes());
                    fingerprint = mix_fingerprint(fingerprint, &step.level.to_le_bytes());
                }
            }
        }
        let legal_actions =
            LegalActionSet::from_canonical_commands(LegalActionFingerprint(fingerprint), commands);
        let mut turn_order = std::iter::once(UnitId(0))
            .chain(allies.iter().map(|ally| ally.unit))
            .chain(hostiles.iter().map(|hostile| hostile.unit))
            .collect::<Vec<_>>();
        turn_order.sort_unstable();

        DecisionRequest {
            controller: PlayerSeat(1),
            group: Some(AiGroupId("benchmark".to_owned())),
            kind: AiDecisionKind::TurnAction,
            observation: AiObservation {
                actor: benchmark_ally(UnitId(0), position(HexCoord::ORIGIN)),
                allies,
                hostiles,
                turn_order,
                round: 7,
                effects: Vec::new(),
                traversal,
            },
            legal_actions,
            cell_choices: None,
        }
    }

    fn selection_fingerprint(selection: &AiSelection) -> u64 {
        match selection {
            AiSelection::Action(key) => {
                let fingerprint = mix_fingerprint(
                    14_695_981_039_346_656_037,
                    &key.fingerprint().0.to_le_bytes(),
                );
                mix_fingerprint(fingerprint, &key.ordinal().to_le_bytes())
            }
            AiSelection::Cells(selection) => {
                let mut fingerprint = mix_fingerprint(
                    14_695_981_039_346_656_037,
                    &selection.fingerprint().0.to_le_bytes(),
                );
                for cell in &selection.cells {
                    fingerprint = mix_fingerprint(fingerprint, &cell.q().to_le_bytes());
                    fingerprint = mix_fingerprint(fingerprint, &cell.r().to_le_bytes());
                }
                fingerprint
            }
        }
    }

    #[test]
    #[ignore = "manual release-mode radius/team AI decision acceptance benchmark"]
    fn baseline_ai_radius_team_matrix_release_benchmark() {
        for radius in [12, 20, 40] {
            for team_size in [1, 3, 6] {
                let request = benchmark_request(radius, team_size);
                assert_eq!(
                    request,
                    benchmark_request(radius, team_size),
                    "radius {radius} {team_size}v{team_size} request was not deterministic"
                );
                let mut algorithm = BaselineAlgorithm;
                let expected = algorithm.select(&request);
                let expected_fingerprint = selection_fingerprint(&expected);
                let mut samples = Vec::with_capacity(100);
                for _ in 0..100 {
                    let started = Instant::now();
                    let selected =
                        std::hint::black_box(algorithm.select(std::hint::black_box(&request)));
                    samples.push(started.elapsed());
                    assert_eq!(selected, expected);
                    assert_eq!(selection_fingerprint(&selected), expected_fingerprint);
                    assert!(matches!(
                        selected,
                        AiSelection::Action(key)
                            if key.fingerprint() == request.legal_actions.fingerprint()
                    ));
                }
                samples.sort_unstable();
                let median = samples.get(49).copied().unwrap_or(Duration::MAX);
                let p95 = samples.get(94).copied().unwrap_or(Duration::MAX);
                let worst = samples.get(99).copied().unwrap_or(Duration::MAX);
                eprintln!(
                    "AI_BENCH radius={radius} teams={team_size}v{team_size} decisions=100 \
                     request_fingerprint={} selection_fingerprint={expected_fingerprint} \
                     median_us={} p95_us={} worst_us={}",
                    request.legal_actions.fingerprint().0,
                    median.as_micros(),
                    p95.as_micros(),
                    worst.as_micros(),
                );

                if radius == 40 {
                    let (p95_budget, worst_budget) = if cfg!(debug_assertions) {
                        (Duration::from_millis(250), Duration::from_millis(500))
                    } else {
                        (Duration::from_millis(50), Duration::from_millis(100))
                    };
                    assert!(
                        p95 < p95_budget && worst < worst_budget,
                        "radius-40 {team_size}v{team_size} exceeded AI budgets: \
                         p95={p95:?}, worst={worst:?}"
                    );
                }
            }
        }
    }
}
