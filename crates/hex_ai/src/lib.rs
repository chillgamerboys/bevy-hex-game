//! Pure contracts for deterministic, replaceable enemy decision algorithms.
//!
//! This crate knows what an algorithm may observe and what it may select. It does
//! not know combat legality and cannot mutate the simulation. `hex_combat` owns both:
//! it builds a canonically ordered [`LegalActionSet`], invokes a registered
//! [`AiAlgorithm`], validates the returned [`ActionKey`], and sends the matched
//! [`GameCommand`] through the ordinary command applier.

use bevy_ecs::{prelude::Component, reflect::ReflectComponent};
use bevy_reflect::Reflect;
use hex_core::{
    EffectPayload, GameCommand, KnowledgeState, LatticeCoord, PlayerSeat, TilePos, UnitId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

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
}

/// Mutable, session-scoped implementation selected by an AI profile.
///
/// Implementations may retain deterministic actor/group state. The runtime clears
/// algorithm instances on gameplay teardown; persistence belongs to Wave 5.
pub trait AiAlgorithm: Send + Sync + 'static {
    /// Selects one request-scoped action key without mutating game state.
    fn select(&mut self, request: &DecisionRequest) -> ActionKey;
}

/// Shipped deterministic combat policy.
#[derive(Debug, Default)]
pub struct BaselineAlgorithm;

impl AiAlgorithm for BaselineAlgorithm {
    fn select(&mut self, request: &DecisionRequest) -> ActionKey {
        let actions = request.legal_actions.actions();
        let fallback = actions
            .iter()
            .find(|action| matches!(action.command, GameCommand::EndTurn { .. }))
            .or_else(|| actions.first())
            .map(|action| action.key)
            .unwrap_or_else(|| ActionKey::from_parts(request.legal_actions.fingerprint(), 0));

        match request.kind {
            AiDecisionKind::ChooseDisables => choose_cells(request, false).unwrap_or(fallback),
            AiDecisionKind::ChooseRestores => choose_cells(request, true).unwrap_or(fallback),
            AiDecisionKind::TurnAction => choose_turn_action(request).unwrap_or(fallback),
        }
    }
}

fn choose_cells(request: &DecisionRequest, restoring: bool) -> Option<ActionKey> {
    request
        .legal_actions
        .actions()
        .iter()
        .filter_map(|action| {
            let cells = match (&action.command, restoring) {
                (GameCommand::ChooseDisables { cells, .. }, false)
                | (GameCommand::ChooseRestores { cells, .. }, true) => cells,
                _ => return None,
            };
            let lattice_unit = match action.command {
                GameCommand::ChooseRestores { target, .. } => target,
                _ => action.command.unit(),
            };
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
            let mut ranks: Vec<(u8, u16, LatticeCoord)> = cells
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
            ranks.sort_unstable();
            Some((ranks, action.key))
        })
        .min_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, key)| key)
}

fn choose_turn_action(request: &DecisionRequest) -> Option<ActionKey> {
    let actions = request.legal_actions.actions();
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

    actions
        .iter()
        .filter_map(|action| match &action.command {
            GameCommand::MoveAlong { path, .. } => {
                let endpoint = path.last()?;
                let used = u32::try_from(path.len().saturating_sub(1)).unwrap_or(u32::MAX);
                let nearest = request
                    .observation
                    .hostiles
                    .iter()
                    .filter_map(|hostile| {
                        route_to_melee(&request.observation.traversal, *endpoint, hostile.position)
                            .map(|remaining| {
                                (used.saturating_add(remaining), hostile.unit, remaining)
                            })
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

fn route_to_melee(
    traversal: &[AiTraversalObservation],
    start: TilePos,
    target: TilePos,
) -> Option<u32> {
    let by_position: BTreeMap<TilePos, &AiTraversalObservation> = traversal
        .iter()
        .map(|surface| (surface.position, surface))
        .collect();
    let target_edges = &by_position.get(&target)?.neighbors;
    let mut distance = BTreeMap::from([(start, 0u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(at) = frontier.pop_front() {
        let cost = *distance.get(&at)?;
        let surface = by_position.get(&at)?;
        if surface.neighbors.contains(&target) && target_edges.contains(&at) {
            return Some(cost);
        }
        for &next in &surface.neighbors {
            if distance.contains_key(&next) {
                continue;
            }
            distance.insert(next, cost.saturating_add(1));
            frontier.push_back(next);
        }
    }
    None
}

/// Why a requested AI selection did not produce its named command.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiDecisionFailure {
    /// The algorithm returned a key from another request fingerprint.
    StaleFingerprint,
    /// The fingerprint matched but no action had the returned ordinal.
    UnknownAction,
}

/// Development trace for one deterministic AI dispatch.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiDecisionTrace {
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
    /// Fingerprint of the offered legal set.
    pub fingerprint: LegalActionFingerprint,
    /// Returned key.
    pub selected: ActionKey,
    /// Failure when the key could not resolve.
    pub failure: Option<AiDecisionFailure>,
}

#[cfg(test)]
mod tests {
    use hex_core::{HexCoord, KnowledgeState};

    use super::*;

    struct FirstAlgorithm;

    impl AiAlgorithm for FirstAlgorithm {
        fn select(&mut self, request: &DecisionRequest) -> ActionKey {
            request.legal_actions.actions().first().map_or_else(
                || ActionKey::from_parts(request.legal_actions.fingerprint(), 0),
                |action| action.key,
            )
        }
    }

    struct LastAlgorithm;

    impl AiAlgorithm for LastAlgorithm {
        fn select(&mut self, request: &DecisionRequest) -> ActionKey {
            request.legal_actions.actions().last().map_or_else(
                || ActionKey::from_parts(request.legal_actions.fingerprint(), 0),
                |action| action.key,
            )
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
}
