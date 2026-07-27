//! Ids for the lattice — a character's local hex grid of gems, fusions and spells.
//!
//! [`LatticeCoord`] is deliberately a *distinct* type from
//! [`HexCoord`](crate::HexCoord), and deliberately offers **no world conversion**:
//! a character's lattice is an abstract arrangement of gems, not a place on the
//! map, and the two spaces must never be confusable. Keying map logic on a lattice
//! coordinate (or the reverse) is a class of silent geometric bug worth making
//! unrepresentable in the type system.
//!
//! [`SpellId`] and [`EnchantId`] are opaque integer handles in the
//! [`SubstanceId`](crate::SubstanceId) style. All three carry `serde` so a
//! `hex_lattice::LatticeSpec` round-trips to and from `lattices.ron`.

use bevy_reflect::prelude::*;
use hexx::Hex;
use serde::{Deserialize, Serialize};

/// A coordinate on a character's lattice — the abstract hex grid of gems.
///
/// Unlike [`HexCoord`](crate::HexCoord) this exposes **no world/pixel
/// conversion**: the lattice is character-local, not a position on the map.
/// Adjacency is the entire power mechanism of the game's magic, so
/// [`neighbors`](Self::neighbors) is the method that matters.
// `Ord` carries no geometric meaning — a hex grid has no natural order. It exists
// so a coordinate can key a `BTreeMap` and give deterministic iteration for
// casting, saving, and a stable diff. It compares the stored axial pair (`q`,
// then `r`).
#[derive(
    Reflect,
    Serialize,
    Deserialize,
    Debug,
    Default,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
pub struct LatticeCoord {
    q: i32,
    r: i32,
}

impl LatticeCoord {
    /// The centre of the lattice.
    pub const ORIGIN: Self = Self { q: 0, r: 0 };

    /// Builds a coordinate from its axial pair.
    #[must_use]
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// The `q` axial coordinate.
    #[must_use]
    pub const fn q(self) -> i32 {
        self.q
    }

    /// The `r` axial coordinate.
    #[must_use]
    pub const fn r(self) -> i32 {
        self.r
    }

    /// The six adjacent lattice cells, in a fixed direction order.
    #[must_use]
    pub fn neighbors(self) -> [Self; 6] {
        Hex::new(self.q, self.r)
            .all_neighbors()
            .map(|hex| Self { q: hex.x, r: hex.y })
    }

    /// Whether `other` is one of the six cells adjacent to this one.
    #[must_use]
    pub fn is_adjacent(self, other: Self) -> bool {
        self.neighbors().contains(&other)
    }

    /// The number of steps between this coordinate and `other`.
    #[must_use]
    pub fn distance(self, other: Self) -> u32 {
        Hex::new(self.q, self.r).unsigned_distance_to(Hex::new(other.q, other.r))
    }
}

/// Opaque identity of a spell.
///
/// Assigned from sorted spell names by `hex_assets`; never written into files or
/// saves (names are), so it is a session-local handle.
#[derive(
    Reflect,
    Serialize,
    Deserialize,
    Debug,
    Default,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
pub struct SpellId(pub u16);

/// A per-battle handle to an active enchantment.
///
/// Allocated from a monotonic counter in `hex_lattice::LatticeState`, never reused
/// within a battle, so it is a stable key for the enchantment's gem locks and for
/// the record emitted when it breaks.
#[derive(
    Reflect,
    Serialize,
    Deserialize,
    Debug,
    Default,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
pub struct EnchantId(pub u32);
