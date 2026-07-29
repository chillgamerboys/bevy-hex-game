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

/// One authorized lattice cell in an observation.
///
/// Unknown hostile facts remain [`None`]. Stable content names are used for kinds
/// rather than session-local IDs.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiLatticeCell {
    /// Stable cell position.
    pub coord: LatticeCoord,
    /// Stable authored kind name when authorized.
    pub kind: Option<String>,
    /// Whether the cell is disabled when authorized.
    pub disabled: Option<bool>,
    /// Current mana when authorized.
    pub mana: Option<u16>,
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

    fn request() -> DecisionRequest {
        let actor = AiAlliedUnit {
            unit: UnitId(1),
            position: TilePos::new(HexCoord::ORIGIN, 1),
            downed: false,
            lattice: AiLatticeObservation {
                capacity: Some(1),
                cells: vec![AiLatticeCell {
                    coord: LatticeCoord::ORIGIN,
                    kind: Some("Spell".to_owned()),
                    disabled: Some(false),
                    mana: None,
                }],
            },
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
}
