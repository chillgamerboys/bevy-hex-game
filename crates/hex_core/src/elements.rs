//! Element identity — a **temporary bridge owned by HEX-7**.
//!
//! HEX-7 ("Elements and spells as content") is the ticket that lands the real
//! [`ElementId`]: assigned from sorted names in `elements.ron` exactly like
//! [`SubstanceId`](crate::SubstanceId), with the six-element wheel, opposition
//! (index arithmetic over the wheel array) and fusion recipes as *data*. The
//! lattice engine (HEX-8) is the wave-1 long pole and reaches compilation before
//! HEX-7 merges, so this minimal newtype exists only so `hex_lattice` can be
//! built and property-tested now.
//!
//! When HEX-7 lands, **delete this module** and re-point `hex_lattice` at the real
//! type. The shape — an opaque `u16` newtype in the `SubstanceId` style — makes that
//! a near-drop-in, with **one caveat**: HEX-7's `ElementId` must also derive
//! `Serialize`/`Deserialize` (which `SubstanceId` does **not**), because
//! `hex_lattice::CellKind` serializes it into a `LatticeSpec`. No code ever matches
//! on a specific element.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

/// Opaque identity of an element.
///
/// Assigned from sorted element names by `hex_assets`. The *hand-authored*
/// `lattices.ron` references elements by name (resolved to this id at load — that
/// is HEX-12's job), but this id is what a `LatticeSpec` serializes once resolved,
/// so it does appear in the engine's serde form. See the [module
/// documentation](self): this is a HEX-7-owned bridge the lattice engine builds
/// against.
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
