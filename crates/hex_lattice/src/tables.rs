//! The content lookup surface: how the engine reads spell and fusion definitions.
//!
//! These traits are defined *here* and implemented by `hex_assets` over the RON
//! content in a later ticket, so the engine depends on no content crate and blocks
//! on no content work. The property tests supply trivial in-memory implementations.
//!
//! Ids are opaque: no method here, and no code in this crate, ever matches on a
//! specific element or spell. Element *opposition* (the wheel arithmetic that
//! powers recharge-on-opposition gems) is not part of this surface — that is a
//! content effect resolved above the engine, not something `castable` needs.

use hex_core::{ElementId, SpellId};

/// One element requirement of a spell or fusion: `mana` of `element`, pooled from
/// as many adjacent gems and live fusion outputs as it takes to reach that total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    /// The element the adjacent sources must provide.
    pub element: ElementId,
    /// The total this requirement draws, possibly split across several sources.
    ///
    /// A **gem** contributes up to its own mana toward the total; several gems of
    /// the right element can each cover part of it. A **fusion** contributes any
    /// share of the total by scaling its own recipe by that share: each of the
    /// recipe's feeder requirements is multiplied by the amount drawn from that
    /// fusion before being resolved (recursively, if a feeder is itself a fusion).
    /// Drawing 1 unit from a fusion reproduces its recipe's base cost exactly, so a
    /// tier-1 fusion draw is unaffected — only a spell that explicitly asks for more
    /// than one unit of a fused element pays proportionally more of its underlying
    /// feeders. No single fusion ever funds more than one requirement, and a gem
    /// funding a requirement directly is the same one-slot-one-source deal — but a
    /// gem reached only as a fusion's own feeder may fund more than one fusion,
    /// split across them up to its own total mana.
    pub mana: u16,
}

/// How a spell spends the mana it draws.
///
/// The two casting axes the design names: evocations spend *throughput*
/// (recoverable by channelling), enchantments spend *capacity* (mana tied up for
/// as long as the enchantment lasts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Casting {
    /// Drains and consumes the mana. Its cost is throughput.
    Evocation,
    /// Ties the drawn mana up for as long as the enchantment lasts, and is lost if
    /// the enchantment breaks.
    Enchantment {
        /// The flat reduction this enchantment applies to incoming disable counts
        /// while it is active. Zero for a non-defensive enchantment.
        defense: u16,
    },
}

/// Fusion recipes: what a fusion output consumes from its adjacent feeders.
pub trait FusionTable {
    /// The adjacent requirements a fusion producing `output` consumes, or [`None`]
    /// if `output` is a basic element rather than a fusion output.
    ///
    /// Recipes must be acyclic — validated upstream when `elements.ron` loads — but
    /// the engine's own resolution is cycle-safe regardless.
    fn recipe(&self, output: ElementId) -> Option<Vec<Requirement>>;
}

/// Spell definitions the engine reads. `hex_assets` implements this over
/// `spells.ron`.
pub trait SpellTable {
    /// The adjacent gem/fusion requirements this spell needs, as an element
    /// multiset. Its length is the spell's tier — at most six, a full ring.
    fn requirements(&self, spell: SpellId) -> Vec<Requirement>;

    /// How this spell spends the mana it draws.
    fn casting(&self, spell: SpellId) -> Casting;
}

/// The full lookup surface [`castable`](crate::castable) and
/// [`apply_cast`](crate::apply_cast) need. Blanket-implemented for any type that is
/// both a [`FusionTable`] and a [`SpellTable`], so a single content struct can
/// serve both.
pub trait Tables: FusionTable + SpellTable {}

impl<T: FusionTable + SpellTable> Tables for T {}
