//! The inscription: a lattice's fixed arrangement of cells.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use hex_core::{ElementId, LatticeCoord, SpellId};
use serde::{Deserialize, Serialize};

/// What occupies one cell of a lattice.
///
/// A cell is a gem, a fusion, a spell, or an inscribed-but-empty blank. Absence
/// from the [`LatticeSpec`] map means the coordinate is *not part of the lattice*
/// at all; [`Blank`](Self::Blank) means it is part of it but holds nothing.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// A gem holding mana of a single element.
    Gem {
        /// The element this gem stores and supplies to adjacent spells and fusions.
        element: ElementId,
    },
    /// A fusion producing a higher-order element from its adjacent feeders.
    ///
    /// The output is fixed at inscription — a build commitment, not chosen
    /// dynamically from whichever neighbours happen to be live.
    Fusion {
        /// The element this fusion outputs when it is live.
        output: ElementId,
    },
    /// A spell that consumes adjacent mana when cast.
    Spell {
        /// Which spell this cell casts.
        spell: SpellId,
    },
    /// An inscribed but empty cell: capacity with nothing in it.
    Blank,
}

/// The inscription of a lattice: its fixed, battle-invariant arrangement of cells.
///
/// This is the authored half of the inscription/state split (see the crate
/// docs). It is the serde format that `lattices.ron` and the future in-game editor
/// share, so `Serialize` lands with `Deserialize` and a round-trip is identity.
#[derive(Component, Reflect, Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct LatticeSpec {
    cells: BTreeMap<LatticeCoord, CellKind>,
}

impl LatticeSpec {
    /// Builds a spec from its cells.
    #[must_use]
    pub fn new(cells: BTreeMap<LatticeCoord, CellKind>) -> Self {
        Self { cells }
    }

    /// Inserts or replaces a cell, returning `self` so calls can chain.
    #[must_use]
    pub fn with(mut self, coord: LatticeCoord, kind: CellKind) -> Self {
        self.cells.insert(coord, kind);
        self
    }

    /// The cell at `coord`, if the lattice has one there.
    #[must_use]
    pub fn get(&self, coord: LatticeCoord) -> Option<CellKind> {
        self.cells.get(&coord).copied()
    }

    /// Every cell, in coordinate order.
    pub fn cells(&self) -> impl Iterator<Item = (LatticeCoord, CellKind)> + '_ {
        self.cells.iter().map(|(&coord, &kind)| (coord, kind))
    }

    /// How many cells the lattice has — its capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.cells.len()
    }

    /// The neighbours of `coord` that are present in the lattice, in coordinate
    /// order (the fixed direction order sorted by [`LatticeCoord`]).
    #[must_use]
    pub fn present_neighbors(&self, coord: LatticeCoord) -> Vec<(LatticeCoord, CellKind)> {
        let mut found: Vec<(LatticeCoord, CellKind)> = coord
            .neighbors()
            .into_iter()
            .filter_map(|neighbor| self.get(neighbor).map(|kind| (neighbor, kind)))
            .collect();
        found.sort_by_key(|(neighbor, _)| *neighbor);
        found
    }
}
