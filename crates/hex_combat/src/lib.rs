//! The gameplay loop: real time until something happens, then turns.
//!
//! The game plays like Baldur's Gate 3 — you walk around freely, and the moment a
//! hostile is close enough the world starts taking turns. There is **one map** and one
//! set of units either way. [`Mode`](hex_core::Mode) is the switch.
//!
//! # What is provisional
//!
//! Almost all of it. The design has not settled **initiative**, **action economy** or
//! **fight length**, and this crate needs an answer to all three to run at all. So it
//! picks the cheapest defensible one, says so, and puts the numbers in
//! `assets/config/combat.ron` where they are obviously knobs rather than decisions:
//!
//! - **Initiative** is a component with a fixed value, ordered high-to-low. The design
//!   proposes deriving it from lattice size, which could also solve boss action
//!   economy by giving a large lattice several slots in the order. That policy remains
//!   provisional even though lattices now exist.
//! - **A turn** is a movement budget and one action. The design's current preference
//!   is free movement of one or two hexes plus one action; this is that, with the
//!   budget exposed so it can be tried.
//! - **Damage disables lattice cells.** Strikes and damage spells name a count,
//!   defences subtract, and the defender chooses the exact cells through the command
//!   funnel. The configured counts remain provisional balance knobs.
//!
//! **No randomness**, which is not provisional — the design is explicit that
//! uncertainty comes from hidden information rather than dice. Ties in initiative
//! break by stable [`UnitId`](hex_core::UnitId), so the same units always produce
//! the same order.

use bevy::prelude::*;
use hex_assets::{Effect, ManaAxis, Spell};
use hex_core::{AppSystems, AuthoritativeSystems, PerceptionSystems, SimulationRole};

/// What an enemy does with its turn. A placeholder, and says so.
mod ai;
/// Freezes published Bevy facts and projects the pure combat authority.
mod authority_host;
/// The applier: the one place a command becomes a change to the sim.
mod commands;
/// Effects that outlast the action that caused them.
pub mod effects;
/// What a faction knows about a hostile lattice.
pub mod knowledge;
/// Structured outcomes produced by combat resolution.
pub mod outcomes {
    pub use hex_combat_core::outcomes::*;
}
/// Terminal encounter detection and its simulation gate.
pub mod resolution;
/// Pending host-resolved area and terrain spell transactions.
pub mod spell_resolution;
/// Deterministic session combat reporting.
pub mod summary;
/// Whose turn it is, and what they have left.
pub mod turns;

pub use ai::{AiAlgorithmRegistry, AiDecisionTraces, MAX_AI_DECISION_TRACES};
pub use commands::{channel_refusal, delivers_anything, ChannelReadiness, UNDELIVERABLE};
pub use effects::PersistentEffects;
pub use hex_core::Turn;
pub use knowledge::{
    BaseVisibility, FactionLatticeKnowledge, KnownCell, LatticeKnowledge, RevealAll,
};
pub use outcomes::{
    CastBlockReason, CombatData, CombatEvent, CommandRefusal, EncounterOutcome, PartyMoveRefusal,
    RestorationRefusal, RestorationTargetRefusal, UnitData,
};
pub use resolution::{encounter_unresolved, EncounterResolution};
pub use spell_resolution::{SpellResolutionFailure, SpellResolutionState, SpellResolutionStatus};
pub use summary::{
    CombatSummary, CombatTranscriptRecorder, CommandKind, DeliveredEffectKind, UnitCombatSummary,
    COMBAT_SUMMARY_FINGERPRINT_VERSION, MAX_COMBAT_SUMMARY_DETAILS,
};
pub use turns::{Initiative, TurnOrder};

/// Clones the canonical renderer-free combat state for read-only diagnostics.
///
/// This deliberately exposes no mutable authority handle. Behavioral tests may use
/// it to prove that an in-combat assertion was not satisfied by a legacy fallback.
pub fn authority_snapshot(world: &World) -> Result<hex_combat_core::CombatState, String> {
    authority_host::snapshot(world)
}

/// Publishes a complete content-adapter projection to the combat authority.
///
/// This is an explicit synchronization token, not a mutable authority handle. The
/// projection is adopted only after deferred ECS writes settle and passes the same
/// exact-roster validation used by runtime content effects.
pub fn publish_combat_adapter_facts(world: &mut World) -> Result<(), String> {
    authority_host::publish_adapter_facts(world)
}

/// Combat-owned compatibility verdict for spells authored by the Wave 6 creator.
///
/// Asset validation answers whether a spell is structurally coherent. This function
/// answers the narrower runtime question: whether combat delivers the complete promise
/// the creator exposes. Keeping it here makes adding an applier arm and lifting its
/// creator restriction one reviewable change.
pub fn creator_spell_deployability(spell: &Spell) -> Result<(), Vec<String>> {
    let mut issues = Vec::new();
    let area = spell.targeting.shape.can_cover_multiple_voxels();
    let mut delivers = matches!(
        spell.casting,
        hex_assets::CastingAxis::Enchantment { defense } if defense > 0
    );
    if spell.mana != ManaAxis::Fixed {
        issues.push("variable mana is not implemented".to_owned());
    }
    if spell.co_castable {
        issues.push("co-casting is not implemented".to_owned());
    }
    for effect in &spell.effects {
        match effect {
            Effect::DisableHexes {
                targeted: false, ..
            }
            | Effect::Burn { .. }
            | Effect::Impact { .. } => delivers = true,
            Effect::RestoreHexes { .. } | Effect::Reveal { .. } if !area => delivers = true,
            Effect::RestoreHexes { .. } => {
                issues.push("area Restore is not safely delivered".to_owned());
            }
            Effect::Reveal { .. } => {
                issues.push("area Reveal is not safely delivered".to_owned());
            }
            _ => issues.push(format!("effect {effect:?} is not completely delivered")),
        }
    }
    if !delivers {
        issues.push(UNDELIVERABLE.to_owned());
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

/// The order a turn resolves in.
///
/// **Acting has to finish before the turn can pass**, and until this set existed
/// nothing said so. `take_enemy_turn` was in `PausableSystems` alone while
/// `advance_turn` was also in [`AppSystems::Update`], so the two were unordered and
/// could even run in parallel.
///
/// That mattered because acting is half immediate and half deferred: `spend` mutates
/// [`Turn`] in place, but the walk animation goes through `Commands`. Advancing in
/// between saw a turn marked finished with nothing yet attached to say the unit was
/// moving — so the turn passed before the enemy had taken a step.
///
/// A shared set rather than `.before(advance_turn)` because ordering across modules is
/// what sets are for, and Bevy inserts the sync point that makes the deferred half
/// visible at the boundary.
#[derive(SystemSet, Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CombatSystems {
    /// Decide and emit what a unit does with its turn.
    Act,
    /// Drain the command queue: validate, apply, start presentation.
    ///
    /// Its own phase rather than part of [`Self::Act`] so the set boundary
    /// supplies the ordering *and* the sync point between deciding and
    /// applying — the AI's emission is visible to the applier in the same
    /// frame, and the applier's committed presentation is visible to
    /// [`Self::Advance`].
    Apply,
    /// Mark newly downed units and detect a terminal encounter.
    Resolve,
    /// Pass the turn on, once whoever holds it has finished.
    Advance,
}

/// Adds the combat loop.
pub fn plugin(app: &mut App) {
    app.init_resource::<hex_core::InputBindings>()
        .init_resource::<SimulationRole>();
    app.add_message::<CombatEvent>();
    app.configure_sets(
        Update,
        AuthoritativeSystems.run_if(resource_equals(SimulationRole::Authority)),
    );
    app.configure_sets(
        Update,
        (
            CombatSystems::Act
                .after(PerceptionSystems::PublishKnowledge)
                .after(hex_units::TerrainOccupancySystems::Publish)
                .after(hex_units::AuthoredObjectOccupancySystems::Publish),
            CombatSystems::Apply
                .after(hex_units::TerrainOccupancySystems::Publish)
                .after(hex_units::AuthoredObjectOccupancySystems::Publish),
            CombatSystems::Resolve,
            CombatSystems::Advance,
        )
            .chain()
            .in_set(AuthoritativeSystems)
            .in_set(AppSystems::Update),
    );
    app.configure_sets(
        Update,
        hex_core::PausableSystems.run_if(resolution::encounter_unresolved),
    );
    app.add_plugins((
        turns::plugin,
        authority_host::plugin,
        ai::plugin,
        commands::plugin,
        effects::plugin,
        knowledge::plugin,
        resolution::plugin,
        summary::plugin,
    ));
}

#[cfg(test)]
mod creator_tests {
    use super::*;
    use hex_assets::{CastingAxis, GemRequirement, TargetShape, TargetingSpec, Trajectory};

    fn ready_spell(effect: Effect) -> Spell {
        Spell {
            requirements: vec![GemRequirement {
                element: "Fire".to_owned(),
                mana: 1,
            }],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 3,
                reach: hex_assets::TargetingReach::Ranged,
                shape: TargetShape::Single,
                trajectory: Trajectory::None,
            },
            effects: vec![effect],
        }
    }

    #[test]
    fn creator_delivery_accepts_the_supported_single_target_behavior_set() {
        for effect in [
            Effect::DisableHexes {
                count: 1,
                targeted: false,
            },
            Effect::Burn { turns: 2 },
            Effect::RestoreHexes { count: 1 },
            Effect::Reveal { tier: 1 },
        ] {
            assert!(creator_spell_deployability(&ready_spell(effect)).is_ok());
        }

        let targeted = ready_spell(Effect::DisableHexes {
            count: 1,
            targeted: true,
        });
        assert!(creator_spell_deployability(&targeted).is_err());
    }

    #[test]
    fn creator_delivery_admits_area_disable_burn_and_impact() {
        let mut spell = ready_spell(Effect::Impact {
            element: "Fire".to_owned(),
            power: 2,
        });
        spell.targeting.shape = TargetShape::Sphere { radius: 2 };
        spell.effects = vec![
            Effect::DisableHexes {
                count: 3,
                targeted: false,
            },
            Effect::Burn { turns: 2 },
            Effect::Impact {
                element: "Fire".to_owned(),
                power: 2,
            },
        ];

        assert!(
            creator_spell_deployability(&spell).is_ok(),
            "the supported area transaction should be Creator-deployable"
        );

        spell.effects = vec![Effect::Impact {
            element: "Fire".to_owned(),
            power: 2,
        }];
        assert!(
            creator_spell_deployability(&spell).is_ok(),
            "an impact-only area spell still delivers terrain behavior"
        );
    }

    #[test]
    fn creator_delivery_keeps_area_restore_and_reveal_fail_closed() {
        for effect in [
            Effect::RestoreHexes { count: 1 },
            Effect::Reveal { tier: 1 },
        ] {
            let mut spell = ready_spell(effect);
            spell.targeting.shape = TargetShape::Sphere { radius: 2 };
            let issues = creator_spell_deployability(&spell)
                .expect_err("unsettled area information policy must fail closed");
            assert!(
                issues.iter().any(|issue| issue.contains("area")),
                "{issues:?}"
            );
        }
    }

    #[test]
    fn creator_delivery_accepts_trajectories_but_rejects_unimplemented_axes() {
        let mut spell = ready_spell(Effect::Burn { turns: 1 });
        spell.targeting.trajectory = Trajectory::Direct;
        spell.co_castable = true;
        spell.mana = ManaAxis::Variable;
        let issues = creator_spell_deployability(&spell).expect_err("unsupported axes must fail");
        assert!(issues.iter().any(|issue| issue.contains("co-casting")));
        assert!(issues.iter().any(|issue| issue.contains("variable mana")));
        assert!(!issues.iter().any(|issue| issue.contains("trajectory")));
    }
}
