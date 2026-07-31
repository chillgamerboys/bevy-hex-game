//! Pure, deterministic combat authority.
//!
//! This crate owns combat truth without constructing a Bevy [`App`](bevy_ecs).
//! Frozen arena, roster, rules, and observation inputs are reduced by ordered
//! [`GameCommand`](hex_core::GameCommand)s into serializable state and canonical
//! events. ECS entities, animation, UI, AI, and renderer state are projections or
//! command producers; none are inputs to legality.

pub mod authority;
pub mod outcomes;

pub use authority::{
    ArenaSnapshot, CombatCase, CombatLattice, CombatMetrics, CombatRunSnapshot, CombatState,
    CombatTermination, CombatUnit, ControllerInput, ElementNames, LatticeSnapshot, RulesProfile,
    RunBounds, TurnStateSnapshot,
};
pub use outcomes::{
    CastBlockReason, CombatData, CombatEvent, CommandRefusal, EncounterOutcome, PartyMoveRefusal,
    RestorationRefusal, UnitData,
};
