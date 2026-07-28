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
//!   proposes deriving it from lattice size, which would also solve boss action
//!   economy by giving a large lattice several slots in the order. Not built, because
//!   lattices do not exist.
//! - **A turn** is a movement budget and one action. The design's current preference
//!   is free movement of one or two hexes plus one action; this is that, with the
//!   budget exposed so it can be tried.
//! - **Nothing deals damage.** Damage disables lattice hexes, and there are no
//!   lattices, so an attack here is an animation and a log line. Building a stand-in
//!   damage model would bake in the numbers the design explicitly has not chosen.
//!
//! **No randomness**, which is not provisional — the design is explicit that
//! uncertainty comes from hidden information rather than dice. Ties in initiative
//! break by stable [`UnitId`](hex_core::UnitId), so the same units always produce
//! the same order.

use bevy::prelude::*;
use hex_core::AppSystems;

/// What an enemy does with its turn. A placeholder, and says so.
mod ai;
/// The applier: the one place a command becomes a change to the sim.
mod commands;
/// Effects that outlast the action that caused them.
pub mod effects;
/// What a faction knows about a hostile lattice.
pub mod knowledge;
/// Whose turn it is, and what they have left.
pub mod turns;

pub use commands::{delivers_anything, UNDELIVERABLE};
pub use effects::PersistentEffects;
pub use hex_core::Turn;
pub use knowledge::{BaseVisibility, FactionKnowledge, KnownCell, LatticeKnowledge, RevealAll};
pub use turns::{Initiative, TurnOrder};

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
    /// Pass the turn on, once whoever holds it has finished.
    Advance,
}

/// Adds the combat loop.
pub fn plugin(app: &mut App) {
    app.configure_sets(
        Update,
        (
            CombatSystems::Act,
            CombatSystems::Apply,
            CombatSystems::Advance,
        )
            .chain()
            .in_set(AppSystems::Update),
    );
    app.add_plugins((
        turns::plugin,
        ai::plugin,
        commands::plugin,
        effects::plugin,
        knowledge::plugin,
    ));
}
