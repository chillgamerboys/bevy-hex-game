//! The command funnel's vocabulary: what may be asked of the sim, and by whom.
//!
//! Every mutation of sim state — moving, striking, ending a turn — is expressed
//! as a [`GameCommand`], stamped with the [`PlayerSeat`] that issued it, and
//! pushed onto the [`CommandQueue`]. One applier (in `hex_combat`) drains the
//! queue, validates each command against the rules, and either applies it or
//! drops it with a logged reason. Input handlers and the AI *emit*; they no
//! longer mutate.
//!
//! That single choke point is what makes a replay possible — the sim's entire
//! input is the ordered command sequence — and what makes co-op honest: a
//! command from the wrong seat dies in validation rather than in a code review.
//!
//! # Why a queue resource and not a `Message`
//!
//! [`TerrainEdit`](crate::TerrainEdit) crosses its crate boundary as a
//! `Message`, and that is right for it: edits are independent, any number of
//! readers may care, and one lost to a frame boundary is a designer
//! inconvenience. Commands are none of those things. They must be consumed
//! exactly once, by exactly one applier, in exactly the order issued — and a
//! `Message` guarantees none of that: delivery is per-reader cursors over a
//! double-buffered queue that ages entries out after two frames whether or
//! not anything read them. The moment an emitter and the applier disagree
//! about a frame — a schedule change, a run condition, a headless test
//! driving frames by hand — messages quietly vanish. A resource with an
//! explicit drain cannot lose one.

use std::collections::VecDeque;

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

use crate::hex::Sextant;
use crate::unit_ids::{PlayerSeat, UnitId};
use crate::voxel::TilePos;

/// One thing a unit can be asked to do.
///
/// Commands speak sim vocabulary — [`UnitId`] and [`TilePos`], never `Entity`
/// or world-space — so a recorded sequence means the same thing on every run
/// and in every save. The applier grounds them against the live world and
/// refuses the ones that no longer make sense.
///
/// The last three variants are the design's future verbs, defined now so the
/// wire format is stable. The applier rejects them loudly; see each variant
/// for what it waits on.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum GameCommand {
    /// Walk this exact surface path, whose first step is where the unit stands.
    ///
    /// The full path rather than a destination, so the applier validates the
    /// route the emitter actually chose — a replayed command cannot re-route
    /// through terrain that has changed and silently mean something else.
    MoveAlong {
        /// Who walks.
        unit: UnitId,
        /// Every surface in order, starting with the current one.
        path: Vec<TilePos>,
    },
    /// Swing at a target within melee reach.
    Strike {
        /// Who swings.
        unit: UnitId,
        /// Who is hit.
        target: UnitId,
    },
    /// Yield the rest of the turn.
    EndTurn {
        /// Whose turn ends.
        unit: UnitId,
    },
    /// Cast a spell. **Not built** — waits on lattices being wired into units
    /// (HEX-12).
    ///
    /// The payload is settled ahead of the implementation on purpose: the
    /// command log is the replay log, so every field is a permanent save
    /// commitment, and two separate tickets need different halves of it.
    /// Later additions arrive as optional serde-default fields or new
    /// variants — never as speculative fields added now.
    Cast {
        /// Who casts.
        unit: UnitId,
        /// Which spell, **by name**. Ids are assigned from sorted names and
        /// are therefore session-local; a name is what survives a save.
        spell: String,
        /// The one positional anchor. A unit target resolves to the voxel it
        /// stands on, so there is one target vocabulary rather than two.
        target: TilePos,
        /// Which way a directed shape points. Line, cone and authored-path
        /// shapes need it; anchored shapes ignore it.
        #[serde(default)]
        facing: Option<Sextant>,
        /// The choice a variable-mana spell requires, absent for fixed ones.
        #[serde(default)]
        mana: Option<u16>,
    },
    /// Sustain a channelled spell. **Not built** — waits on channelling
    /// (HEX-12).
    Channel {
        /// Who channels.
        unit: UnitId,
    },
    /// Choose which lattice hexes damage disables. **Not built** — waits on
    /// the damage model.
    ChooseDisables {
        /// Who chooses.
        unit: UnitId,
    },
}

impl GameCommand {
    /// The unit this command asks to act.
    #[must_use]
    pub fn unit(&self) -> UnitId {
        match *self {
            Self::MoveAlong { unit, .. }
            | Self::Strike { unit, .. }
            | Self::EndTurn { unit }
            | Self::Cast { unit, .. }
            | Self::Channel { unit }
            | Self::ChooseDisables { unit } => unit,
        }
    }
}

/// A command with the seat that issued it.
///
/// The seat is recorded at emission, not derived at application: a replay must
/// re-validate the same claim the live session made, and in co-op "who asked"
/// is exactly the thing being checked.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IssuedCommand {
    /// Who is asking.
    pub seat: PlayerSeat,
    /// What they ask.
    pub command: GameCommand,
}

/// The funnel itself: commands in issue order, awaiting the one applier.
///
/// Push from anywhere; drained only by `hex_combat`'s applier. First in,
/// first applied — the drain order **is** the sim's input order, which is why
/// this is a queue and not a set or a message (see the module docs).
#[derive(Resource, Debug, Default)]
pub struct CommandQueue {
    queue: VecDeque<IssuedCommand>,
}

impl CommandQueue {
    /// Adds a command after everything already waiting.
    pub fn push(&mut self, issued: IssuedCommand) {
        self.queue.push_back(issued);
    }

    /// Takes the oldest waiting command. The applier's loop.
    pub fn pop(&mut self) -> Option<IssuedCommand> {
        self.queue.pop_front()
    }

    /// Whether anything is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Whether a command naming `unit` is already waiting.
    ///
    /// Emitters use this to fold same-frame repeats: two clicks in one frame
    /// are one intent, and without the check the second would survive to the
    /// applier only to die in its busy gate as a warned drop.
    #[must_use]
    pub fn holds_command_for(&self, unit: UnitId) -> bool {
        self.queue
            .iter()
            .any(|issued| issued.command.unit() == unit)
    }

    /// How many commands are waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Forgets everything waiting. Session teardown: unit ids reset between
    /// sessions, so a held-over command would name somebody else's unit.
    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

/// A unit whose presentation is still in flight.
///
/// Maintained by the applier and its sync system in `hex_combat`: inserted
/// when a command commits an animation, removed once the walk or swing has
/// landed. The applier refuses to start new presentation for a busy unit —
/// which is the one rule that used to live as three separate ad-hoc
/// `Transformation` checks.
///
/// A marker in `hex_core` rather than a query on the animation component,
/// because the *rule* ("one thing at a time") is sim vocabulary while the
/// animation is presentation — and because `hex_core` cannot see `hex_anim`.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct Busy;

/// The sim is waiting on a decision from a seat before resolution continues.
///
/// **Placeholder.** Defined now so the vocabulary is stable: choosing which
/// lattice hexes damage disables (the design's one mid-resolution decision)
/// will park the sim behind this marker. Nothing inserts it until the damage
/// model exists.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct PendingDecision;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_is_first_in_first_out() {
        let mut queue = CommandQueue::default();
        queue.push(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::EndTurn { unit: UnitId(1) },
        });
        queue.push(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::EndTurn { unit: UnitId(2) },
        });

        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop().map(|i| i.command.unit()), Some(UnitId(1)));
        assert_eq!(queue.pop().map(|i| i.command.unit()), Some(UnitId(2)));
        assert!(queue.is_empty());
    }

    #[test]
    fn every_variant_names_its_unit() {
        let unit = UnitId(7);
        let commands = [
            GameCommand::MoveAlong {
                unit,
                path: Vec::new(),
            },
            GameCommand::Strike {
                unit,
                target: UnitId(9),
            },
            GameCommand::EndTurn { unit },
            GameCommand::Cast {
                unit,
                spell: "Ember".to_owned(),
                target: TilePos::new(crate::HexCoord::ORIGIN, 1),
                facing: None,
                mana: None,
            },
            GameCommand::Channel { unit },
            GameCommand::ChooseDisables { unit },
        ];
        for command in commands {
            assert_eq!(command.unit(), unit);
        }
    }
}
