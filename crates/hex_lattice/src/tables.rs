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

/// One element requirement of a spell or fusion: a distinct adjacent gem (or live
/// fusion output) of `element` contributing `mana`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    /// The element the adjacent source must provide.
    pub element: ElementId,
    /// How much mana that source contributes to this cast.
    ///
    /// This is the cost when a **gem** satisfies the requirement. When a fusion
    /// satisfies it, its *recipe* is drained instead and this amount is not —
    /// the design scales high-tier spells by recipe complexity, not volume.
    /// Whether the two costs should be validated against each other is a
    /// content-design question deferred to the wiring ticket (HEX-12).
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
