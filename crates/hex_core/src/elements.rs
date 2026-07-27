//! Opaque identities for the element/spell content system.
//!
//! [`ElementId`] and [`SpellId`] are session-local integer handles in the
//! [`SubstanceId`](crate::SubstanceId) style: `hex_assets` assigns them from the
//! **sorted names** in `elements.ron` and `spells.ron`, so the mapping depends only
//! on the set of names and reordering a file never silently rewrites what an id
//! means. Names are what appear in hand-authored content and in saves; these ids are
//! the resolved runtime form.
//!
//! # Why these carry `serde`
//!
//! Unlike [`SubstanceId`](crate::SubstanceId) — which is only ever stored as a voxel
//! component — these ids are serialized into a `hex_lattice::LatticeSpec`
//! (`CellKind::Gem { element }`, `CellKind::Spell { spell }`), the runtime/save form
//! of a lattice. The hand-authored `lattices.ron` references elements and spells by
//! **name**, resolved to these ids at load; the `LatticeSpec` a save writes carries
//! the resolved ids, guarded against content drift by the save's content digests.
//!
//! # No code matches on a specific element
//!
//! The six-element wheel, opposition (index arithmetic over the wheel array) and the
//! fusion recipes are **data** in `elements.ron`, never a Rust `match` on a
//! particular [`ElementId`]. An id is opaque: it names a row in a table and nothing
//! more.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

/// Opaque identity of an element.
///
/// Assigned from sorted element names in `elements.ron` by `hex_assets`, exactly like
/// [`SubstanceId`](crate::SubstanceId). Covers both the six basic elements and the
/// higher-order elements produced by fusions — every element the content defines has
/// one. It is a [`Component`] so a future gem or fusion entity can carry it directly,
/// and derives `serde` because `hex_lattice::CellKind` serializes it into a
/// `LatticeSpec`.
#[derive(
    Component,
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
#[reflect(Component)]
pub struct ElementId(pub u16);

/// Opaque identity of a spell.
///
/// Assigned from sorted spell names in `spells.ron` by `hex_assets`, exactly like
/// [`ElementId`] and [`SubstanceId`](crate::SubstanceId). The hand-authored
/// `lattices.ron` refers to spells by name; this id is the resolved form a
/// `hex_lattice::LatticeSpec` serializes (`CellKind::Spell { spell }`), which is why
/// it derives `serde`.
///
/// Not a [`Component`]: a spell is a cell in an abstract lattice
/// (`hex_lattice::CellKind`), not an ECS entity of its own.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids must round-trip through serde: `hex_lattice::LatticeSpec` serializes
    /// them, so a save that writes a lattice and reads it back has to recover the
    /// same id. (serde_json is hex_core's available dev-dependency; the format is
    /// immaterial — this asserts the derives are present and correct.)
    #[test]
    fn ids_round_trip_through_serde() {
        let element = ElementId(7);
        let encoded = serde_json::to_string(&element).expect("ElementId serializes");
        let decoded: ElementId = serde_json::from_str(&encoded).expect("ElementId deserializes");
        assert_eq!(element, decoded);

        let spell = SpellId(42);
        let encoded = serde_json::to_string(&spell).expect("SpellId serializes");
        let decoded: SpellId = serde_json::from_str(&encoded).expect("SpellId deserializes");
        assert_eq!(spell, decoded);
    }

    /// Ordering is by the wrapped integer, so these ids can key a `BTreeMap` and give
    /// deterministic iteration — the same property the lattice engine relies on.
    #[test]
    fn ids_order_by_their_integer() {
        assert!(ElementId(1) < ElementId(2));
        assert!(SpellId(1) < SpellId(2));
    }
}
