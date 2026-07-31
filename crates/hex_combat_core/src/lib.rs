//! Pure, deterministic combat authority.
//!
//! This crate owns combat truth without constructing a Bevy [`App`](bevy_ecs).
//! Frozen arena, roster, rules, and observation inputs are reduced by ordered
//! [`GameCommand`](hex_core::GameCommand)s into serializable state and canonical
//! events. ECS entities, animation, UI, AI, and renderer state are projections or
//! command producers; none are inputs to legality.

pub mod authority;
pub mod content;
pub mod outcomes;

pub use authority::{
    ActionEconomyPolicy, ArenaSnapshot, CombatCase, CombatLattice, CombatMetrics, CombatMotion,
    CombatRunSnapshot, CombatState, CombatTermination, CombatUnit, CombatUnitProjection,
    CommandBound, ControllerInput, ElementNames, InitiativePolicy, LatticeSnapshot,
    MovementProjectionError, NoProgressBound, RulesProfile, RunBounds, TurnBound,
    TurnStateSnapshot, RULES_PROFILE_VERSION,
};
pub use content::{
    FrozenCasting, FrozenCombatContent, FrozenEffect, FrozenRequirement, FrozenSpell,
    FrozenTargeting,
};
pub use outcomes::{
    CastBlockReason, CombatData, CombatEvent, CommandRefusal, EncounterOutcome, PartyMoveRefusal,
    RestorationRefusal, UnitData,
};
