//! Stable, structured outcomes produced by the combat simulation.
//!
//! [`GameCommand`](hex_core::GameCommand) is the replayable stream of intent. This
//! module is its dual: facts that actually happened after validation and resolution.
//! Presentation consumes these messages, but owns all wording and disclosure policy.
//! No variant stores an `Entity`, a session-local `SpellId`, or a preformatted line.

use bevy::prelude::Message;
use hex_core::{GameCommand, LatticeCoord, PlayerSeat, TilePos, UnitId};
use hex_units::Faction;
use serde::{Deserialize, Serialize};

/// A runtime dependency the command applier needed but could not read.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatData {
    /// Combat policy settings, including damage and divination duration.
    CombatSettings,
    /// The table that resolves spell names and definitions.
    SpellBook,
    /// Cross-content tables used to resolve spell requirements.
    ContentTables,
    /// Terrain substance properties used to validate movement and reach.
    SubstanceTable,
}

/// One per-unit fact required to apply a command.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitData {
    /// The entity record resolved from the stable unit id.
    EntityRecord,
    /// The surface the unit currently stands on.
    Standing,
    /// The unit's traversal body.
    Body,
    /// The side the unit fights for.
    Faction,
    /// The unit's lattice specification and battle state.
    Lattice,
    /// The active turn component.
    Turn,
}

/// Why an otherwise valid spell cell could not supply a cast.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastBlockReason {
    /// The resolved lattice cell was not a spell cell.
    NotASpell,
    /// The spell cell was disabled.
    SpellDisabled,
    /// Adjacent gems and fusions could not meet the spell's requirements.
    Unsatisfiable,
}

/// A closed, serializable reason why the command applier rejected an intent.
///
/// The refused [`GameCommand`] accompanies this value in
/// [`CombatEvent::CommandRefused`], so command-specific coordinates, paths, spell
/// names, and targets remain available without duplicating them in every variant.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum CommandRefusal {
    /// The command's acting unit was not registered.
    UnknownUnit,
    /// The issuing seat did not own the acting unit.
    WrongSeat {
        /// The seat recorded on the command.
        issued_by: PlayerSeat,
        /// The seat recorded on the unit.
        owned_by: PlayerSeat,
    },
    /// This command is legal only while combat is active.
    CombatOnly,
    /// Another unit currently owns the turn.
    NotCurrentTurn {
        /// The current actor, or `None` when the order is empty.
        current: Option<UnitId>,
    },
    /// Resolution is waiting for an earlier decision.
    DecisionPending {
        /// The unit whose answer is required.
        decider: UnitId,
    },
    /// A required combat-wide resource was unavailable.
    MissingCombatData {
        /// Which resource was absent.
        data: CombatData,
    },
    /// A required fact was absent from a unit.
    MissingUnitData {
        /// The unit whose fact was absent.
        unit: UnitId,
        /// Which fact was absent.
        data: UnitData,
    },
    /// The acting unit was still completing presentation for an earlier command.
    Busy,
    /// The supplied movement path was not a complete walkable route from the unit.
    InvalidPath,
    /// The route cost more movement than remained.
    MovementBudgetExceeded {
        /// The validated route cost.
        cost: u32,
        /// The unit's remaining budget.
        remaining: u32,
    },
    /// A strike named a unit that was not registered.
    UnknownTarget {
        /// The missing target.
        target: UnitId,
    },
    /// A damaging command named a unit that has already gone down.
    TargetDowned {
        /// The downed target.
        target: UnitId,
    },
    /// A strike target was not hostile to its attacker.
    TargetNotHostile {
        /// The rejected target.
        target: UnitId,
    },
    /// A strike target was outside bidirectional melee reach.
    TargetOutOfMeleeReach {
        /// The unreachable target.
        target: UnitId,
    },
    /// The unit had no active turn to spend or end.
    NoTurn,
    /// The unit had already spent its action.
    ActionAlreadySpent,
    /// A cast named no loaded spell.
    UnknownSpell {
        /// The stable spell name from the command.
        spell: String,
    },
    /// The spell catalog contained a name without a definition.
    MissingSpellDefinition {
        /// The stable spell name from the command.
        spell: String,
    },
    /// None of the spell's authored outcomes are currently implemented.
    UndeliverableSpell {
        /// The stable spell name from the command.
        spell: String,
    },
    /// A directional spell omitted its facing.
    MissingFacing {
        /// The stable spell name from the command.
        spell: String,
    },
    /// A spell anchor was outside the caster's resolved range.
    TargetOutOfRange {
        /// The stable spell name from the command.
        spell: String,
        /// The rejected anchor.
        target: TilePos,
    },
    /// The spell's authored shape could not resolve.
    ShapeUnresolved {
        /// The stable spell name from the command.
        spell: String,
        /// The attempted anchor.
        target: TilePos,
    },
    /// The caster's faction had not observed the spell anchor.
    TargetUnobserved {
        /// The stable spell name from the command.
        spell: String,
        /// The unobserved anchor.
        target: TilePos,
    },
    /// The caster's lattice did not contain the named spell.
    SpellNotInscribed {
        /// The stable spell name from the command.
        spell: String,
    },
    /// The spell was inscribed but its lattice could not currently cast it.
    CastBlocked {
        /// The stable spell name from the command.
        spell: String,
        /// The exact lattice-engine refusal.
        reason: CastBlockReason,
    },
    /// The payment plan changed before it could commit.
    CastPlanStale {
        /// The stable spell name from the command.
        spell: String,
    },
    /// Channelling is part of the command vocabulary but has no applier yet.
    ChannelUnavailable,
    /// A disable answer arrived while no decision was open.
    NoPendingDecision,
    /// The answer came from a different unit than the open decision named.
    WrongDecisionUnit {
        /// The unit whose answer is required.
        expected: UnitId,
    },
    /// The answer named too many or too few cells.
    WrongDisableCount {
        /// The number of live cells the answer owed.
        expected: u32,
        /// The number supplied by the command.
        actual: u32,
    },
    /// The answer named a coordinate outside the deciding unit's lattice.
    CellOutsideLattice {
        /// The invalid coordinate.
        cell: LatticeCoord,
    },
    /// The answer named the same coordinate more than once.
    DuplicateCell {
        /// The repeated coordinate.
        cell: LatticeCoord,
    },
    /// The answer tried to spend a cell that was already disabled.
    CellAlreadyDisabled {
        /// The already-disabled coordinate.
        cell: LatticeCoord,
    },
}

/// One structured fact produced by successful combat resolution or command rejection.
#[derive(Message, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum CombatEvent {
    /// A spell was paid for and committed.
    Cast {
        /// The caster.
        caster: UnitId,
        /// The stable content name, never a session-local spell id.
        spell: String,
        /// The spell's positional anchor.
        target: TilePos,
    },
    /// A melee strike committed.
    Strike {
        /// The attacking unit.
        attacker: UnitId,
        /// The struck unit.
        target: UnitId,
    },
    /// Damage parked resolution until the defender chooses exact cells.
    DecisionOpened {
        /// The unit choosing from its own lattice.
        decider: UnitId,
        /// The unit responsible for the hit.
        source: UnitId,
        /// The post-defence number of cells owed.
        count: u16,
    },
    /// Defence prevented some or all of an incoming hit.
    DamagePrevented {
        /// The unit responsible for the hit.
        source: UnitId,
        /// The defending unit.
        target: UnitId,
        /// The exact amount removed from the incoming count.
        amount: u16,
    },
    /// A defender's exact chosen cells were disabled.
    HexesDisabled {
        /// The unit responsible for the hit.
        source: UnitId,
        /// The damaged unit.
        target: UnitId,
        /// Exact lattice coordinates, in the answer's recorded order.
        cells: Vec<LatticeCoord>,
    },
    /// Disabling a funding gem broke an enchantment.
    EnchantmentBroken {
        /// The unit whose enchantment broke.
        unit: UnitId,
        /// The stable spell name, or `None` only in an incomplete headless harness
        /// with no matching spell catalog entry.
        spell: Option<String>,
        /// Locked mana destroyed with the enchantment.
        burned_mana: u16,
        /// The disabled cell that broke it.
        trigger: LatticeCoord,
    },
    /// A cast booked a Burn effect.
    BurnApplied {
        /// The unit that caused the Burn.
        source: UnitId,
        /// The burning unit.
        target: UnitId,
        /// How many of the target's turns the Burn lasts.
        turns: u16,
    },
    /// A Burn advanced on a newly granted turn.
    BurnTicked {
        /// The first effect source used for deterministic attribution.
        source: UnitId,
        /// The burning unit.
        target: UnitId,
        /// How many cells the aggregated Burn tick demands.
        count: u16,
    },
    /// Divination revealed a full lattice for a bounded duration.
    Revealed {
        /// The faction that learned the facts.
        viewer: Faction,
        /// The revealed unit.
        subject: UnitId,
        /// Every revealed coordinate in stable lattice order.
        cells: Vec<LatticeCoord>,
        /// Further round rollovers the reveal survives.
        rounds: u32,
    },
    /// A unit's entire lattice became disabled.
    Downed {
        /// The downed unit.
        unit: UnitId,
    },
    /// The command applier rejected an intent without mutating for it.
    CommandRefused {
        /// The exact stable command that was rejected.
        command: GameCommand,
        /// The closed typed reason.
        refusal: CommandRefusal,
    },
}

#[cfg(test)]
mod tests {
    use hex_core::HexCoord;

    use super::*;

    #[test]
    fn every_command_refusal_variant_round_trips() {
        let cell = LatticeCoord::new(2, -1);
        let target = TilePos::new(HexCoord::ORIGIN, 1);
        let variants = vec![
            CommandRefusal::UnknownUnit,
            CommandRefusal::WrongSeat {
                issued_by: PlayerSeat(1),
                owned_by: PlayerSeat(2),
            },
            CommandRefusal::CombatOnly,
            CommandRefusal::NotCurrentTurn {
                current: Some(UnitId(9)),
            },
            CommandRefusal::DecisionPending { decider: UnitId(3) },
            CommandRefusal::MissingCombatData {
                data: CombatData::CombatSettings,
            },
            CommandRefusal::MissingCombatData {
                data: CombatData::SpellBook,
            },
            CommandRefusal::MissingCombatData {
                data: CombatData::ContentTables,
            },
            CommandRefusal::MissingCombatData {
                data: CombatData::SubstanceTable,
            },
            CommandRefusal::MissingUnitData {
                unit: UnitId(1),
                data: UnitData::EntityRecord,
            },
            CommandRefusal::MissingUnitData {
                unit: UnitId(1),
                data: UnitData::Standing,
            },
            CommandRefusal::MissingUnitData {
                unit: UnitId(1),
                data: UnitData::Body,
            },
            CommandRefusal::MissingUnitData {
                unit: UnitId(1),
                data: UnitData::Faction,
            },
            CommandRefusal::MissingUnitData {
                unit: UnitId(1),
                data: UnitData::Lattice,
            },
            CommandRefusal::MissingUnitData {
                unit: UnitId(1),
                data: UnitData::Turn,
            },
            CommandRefusal::Busy,
            CommandRefusal::InvalidPath,
            CommandRefusal::MovementBudgetExceeded {
                cost: 5,
                remaining: 4,
            },
            CommandRefusal::UnknownTarget { target: UnitId(2) },
            CommandRefusal::TargetDowned { target: UnitId(2) },
            CommandRefusal::TargetNotHostile { target: UnitId(2) },
            CommandRefusal::TargetOutOfMeleeReach { target: UnitId(2) },
            CommandRefusal::NoTurn,
            CommandRefusal::ActionAlreadySpent,
            CommandRefusal::UnknownSpell {
                spell: "Unknown".to_owned(),
            },
            CommandRefusal::MissingSpellDefinition {
                spell: "Ember".to_owned(),
            },
            CommandRefusal::UndeliverableSpell {
                spell: "Daylight".to_owned(),
            },
            CommandRefusal::MissingFacing {
                spell: "Flamethrower".to_owned(),
            },
            CommandRefusal::TargetOutOfRange {
                spell: "Ember".to_owned(),
                target,
            },
            CommandRefusal::ShapeUnresolved {
                spell: "Ember".to_owned(),
                target,
            },
            CommandRefusal::TargetUnobserved {
                spell: "Ember".to_owned(),
                target,
            },
            CommandRefusal::SpellNotInscribed {
                spell: "Ember".to_owned(),
            },
            CommandRefusal::CastBlocked {
                spell: "Ember".to_owned(),
                reason: CastBlockReason::NotASpell,
            },
            CommandRefusal::CastBlocked {
                spell: "Ember".to_owned(),
                reason: CastBlockReason::SpellDisabled,
            },
            CommandRefusal::CastBlocked {
                spell: "Ember".to_owned(),
                reason: CastBlockReason::Unsatisfiable,
            },
            CommandRefusal::CastPlanStale {
                spell: "Ember".to_owned(),
            },
            CommandRefusal::ChannelUnavailable,
            CommandRefusal::NoPendingDecision,
            CommandRefusal::WrongDecisionUnit {
                expected: UnitId(4),
            },
            CommandRefusal::WrongDisableCount {
                expected: 2,
                actual: 1,
            },
            CommandRefusal::CellOutsideLattice { cell },
            CommandRefusal::DuplicateCell { cell },
            CommandRefusal::CellAlreadyDisabled { cell },
        ];

        for refusal in variants {
            let encoded = serde_json::to_string(&refusal).expect("a refusal serializes");
            let decoded: CommandRefusal =
                serde_json::from_str(&encoded).expect("a refusal deserializes");
            assert_eq!(decoded, refusal);
        }
    }

    #[test]
    fn every_combat_event_variant_round_trips_without_session_local_ids() {
        let source = UnitId(1);
        let target_unit = UnitId(2);
        let target = TilePos::new(HexCoord::ORIGIN, 1);
        let cell = LatticeCoord::new(1, 0);
        let events = vec![
            CombatEvent::Cast {
                caster: source,
                spell: "Ember".to_owned(),
                target,
            },
            CombatEvent::Strike {
                attacker: source,
                target: target_unit,
            },
            CombatEvent::DecisionOpened {
                decider: target_unit,
                source,
                count: 2,
            },
            CombatEvent::DamagePrevented {
                source,
                target: target_unit,
                amount: 1,
            },
            CombatEvent::HexesDisabled {
                source,
                target: target_unit,
                cells: vec![LatticeCoord::ORIGIN, cell],
            },
            CombatEvent::EnchantmentBroken {
                unit: target_unit,
                spell: Some("Metal Shield".to_owned()),
                burned_mana: 2,
                trigger: cell,
            },
            CombatEvent::BurnApplied {
                source,
                target: target_unit,
                turns: 2,
            },
            CombatEvent::BurnTicked {
                source,
                target: target_unit,
                count: 1,
            },
            CombatEvent::Revealed {
                viewer: Faction::Player,
                subject: target_unit,
                cells: vec![LatticeCoord::ORIGIN, cell],
                rounds: 1,
            },
            CombatEvent::Downed { unit: target_unit },
            CombatEvent::CommandRefused {
                command: GameCommand::Cast {
                    unit: source,
                    spell: "Ember".to_owned(),
                    target,
                    facing: None,
                    mana: None,
                },
                refusal: CommandRefusal::TargetOutOfRange {
                    spell: "Ember".to_owned(),
                    target,
                },
            },
        ];

        for event in events {
            let encoded = serde_json::to_string(&event).expect("an outcome serializes");
            let decoded: CombatEvent =
                serde_json::from_str(&encoded).expect("an outcome deserializes");
            assert_eq!(decoded, event);
        }
    }
}
