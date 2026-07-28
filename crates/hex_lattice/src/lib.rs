//! The **lattice** — the game's core system — as a pure rules engine.
//!
//! A character or enemy is defined by a lattice: a contiguous grid of gems
//! (holding element mana), fusions (combining adjacent elements into higher-order
//! ones), and spells (consuming adjacent mana). Adjacency is the entire power
//! mechanism. There is no HP; damage *disables* hexes, which silences the spells
//! and fusions that depended on them. This crate is the headless, deterministic,
//! serializable engine for all of that, proven by a property suite before any of
//! it is wired into the ECS. See `docs/design/game.md` for the mechanics.
//!
//! # The inscription / state split
//!
//! [`LatticeSpec`] is the **inscription**: the fixed arrangement of cells, authored
//! at level-up and shared with `lattices.ron` and the future in-game editor as a
//! serde format. [`LatticeState`] is the **battle-mutable half**: mana, the
//! disabled set, enchantment locks, and burns. They have different lifetimes, and
//! cloning the small integer [`LatticeState`] is the AI's forward-simulation
//! primitive.
//!
//! # One legality function
//!
//! [`castable`] is the single function that preview, application, and AI all agree
//! on. It returns either a [`CastPlan`] — the exact gem-to-requirement assignment,
//! so the mana drain is unambiguous — or a [`CastBlocked`] reason the UI can show.
//! [`apply_cast`] applies a plan, [`apply_disables`] applies chosen disables (and
//! breaks the enchantments whose gems went down), and [`channel()`] refills.
//!
//! # Content enters through traits
//!
//! Element, spell, and fusion definitions are read through the [`tables`] traits,
//! which `hex_assets` implements over the RON content later. This crate therefore
//! never depends on `hex_assets` and never blocks on content: the property tests
//! supply trivial in-memory implementations.
//!
//! # Determinism is structural
//!
//! Every field is an integer and every collection is a [`BTreeMap`] or
//! [`BTreeSet`]; all iteration is sorted, [`EnchantId`](hex_core::EnchantId)s come
//! from a monotonic counter, and there is no floating point and no RNG anywhere in
//! resolution. Determinism here is a property of the types, which is stronger than
//! a lint.
//!
//! # Deliberately out of scope
//!
//! This engine settles none of the design's open questions: no initiative, no
//! action economy, no fight-length or functional-death threshold, and `channel`
//! is a burst refill only (passive trickle is a policy choice made above the
//! engine). The command funnel, the defender-chooses suspension, death detection,
//! ECS wiring, and applying a spell's outward effect on *another* unit all belong
//! to the crates that consume this one.
//!
//! [`BTreeMap`]: std::collections::BTreeMap
//! [`BTreeSet`]: std::collections::BTreeSet

pub mod cast;
pub mod channel;
pub mod disable;
pub mod spec;
pub mod state;
pub mod tables;

pub use cast::{apply_cast, castable, CastBlocked, CastPlan};
pub use channel::channel;
pub use disable::{apply_disables, resolve_incoming, restore, tick_burns};
pub use spec::{CellKind, LatticeSpec};
pub use state::{ActiveEnchantment, BrokenEnchantment, Burn, LatticeState, LatticeStats};
pub use tables::{Casting, FusionTable, Requirement, SpellTable, Tables};
