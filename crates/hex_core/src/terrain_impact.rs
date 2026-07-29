//! Announcing an elemental effect on the world, and hearing what it did.
//!
//! The second write path beside [`TerrainEdit`](crate::TerrainEdit), and the
//! split between them is not arbitrary — it tracks whether there is a material
//! with an opinion.
//!
//! **Conjuration** names its substance, because nothing is there to respond and
//! the material is the spell's identity. **An elemental effect** names only an
//! element and a strength, because the voxel it reaches already has properties
//! its author defined. Gameplay owns the geometry; the world owns the
//! materiality. See [`systems/casting.md`] and boundary asks G and H.
//!
//! Nothing produces or consumes these yet — they are the agreed shape, landed
//! ahead of the implementation so both sides compile against one vocabulary.
//!
//! [`systems/casting.md`]: https://github.com/chillgamerboys/bevy-hex-game/blob/main/docs/systems/casting.md

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::elements::ElementId;
use crate::voxel::{SubstanceId, TilePos};

/// Identifies one announcement so its outcome can be matched to it.
///
/// Session-local, like every other runtime handle here. A durable log stores
/// its own key and converts elements and substances back to stable names.
#[derive(Reflect, Debug, Default, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerrainBatchId(pub u64);

/// An energetic effect announced over an exact set of voxels.
///
/// The world decides what each material does about it — including nothing.
/// A fully resisted announcement is not an error: the caster committed and the
/// mountain won.
#[derive(Message, Reflect, Debug, Clone, PartialEq, Eq)]
pub struct TerrainImpact {
    /// Dealt by gameplay; echoed back by [`TerrainImpactOutcome`].
    pub batch: TerrainBatchId,
    /// Every exact voxel the effect reaches, **sorted and deduplicated**.
    ///
    /// Canonical form is part of the contract rather than a convenience: it
    /// makes the message content-addressable, and stops replay order depending
    /// on how the publisher happened to build the vector. A consumer rejects a
    /// non-canonical message rather than quietly applying it.
    pub volume: Vec<TilePos>,
    /// Which element arrived. A runtime handle — the authored response table
    /// and any durable log use the stable element *name*.
    pub element: ElementId,
    /// How strong, from the spell's own content.
    pub power: u8,
}

impl TerrainImpact {
    /// Whether `volume` is in the canonical form the contract requires.
    ///
    /// Sorted, with no repeats. A consumer checks this and refuses rather than
    /// letting event order depend on input accidents.
    #[must_use]
    pub fn is_canonical(&self) -> bool {
        self.volume.windows(2).all(|pair| match pair {
            [a, b] => a < b,
            _ => true,
        })
    }
}

/// What the world decided for one voxel an impact reached.
///
/// Explicit rather than inferred: `before`/`after` alone cannot tell empty
/// space from a material that resisted from one whose response was to do
/// nothing, and those are three different things to a player.
#[derive(Reflect, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TerrainImpactDisposition {
    /// There was nothing there to affect.
    NoMaterial,
    /// Material was present and refused the effect.
    Resisted,
    /// Material was present and its response was to stay as it was.
    Unchanged,
    /// Material was removed.
    Cleared,
    /// Material became a different substance.
    Replaced,
}

/// One voxel's result.
#[derive(Reflect, Debug, Copy, Clone, PartialEq, Eq)]
pub struct TerrainVoxelOutcome {
    /// Which voxel.
    pub pos: TilePos,
    /// What the world decided.
    pub disposition: TerrainImpactDisposition,
    /// The substance before, if any.
    pub before: Option<SubstanceId>,
    /// The substance after, if any.
    pub after: Option<SubstanceId>,
}

impl TerrainVoxelOutcome {
    /// Whether the substances agree with the stated disposition.
    ///
    /// The dispositions are not free-form: `NoMaterial` means nothing before
    /// and nothing after, `Resisted` and `Unchanged` mean the same substance
    /// either side, `Cleared` means something became nothing, and `Replaced`
    /// means one substance became a *different* one. Anything else is a
    /// malformed answer.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        match self.disposition {
            TerrainImpactDisposition::NoMaterial => self.before.is_none() && self.after.is_none(),
            TerrainImpactDisposition::Resisted | TerrainImpactDisposition::Unchanged => {
                self.before.is_some() && self.before == self.after
            }
            TerrainImpactDisposition::Cleared => self.before.is_some() && self.after.is_none(),
            TerrainImpactDisposition::Replaced => {
                self.before.is_some() && self.after.is_some() && self.before != self.after
            }
        }
    }
}

/// What every voxel in one announced impact became.
///
/// **An authoritative simulation message, not permission to reveal its
/// payload.** An area reaching into terrain a faction cannot see resolves
/// truthfully, but presentation, logs and faction-facing knowledge filter every
/// entry through observation — otherwise an acknowledgment leaks the material
/// of ground nobody has looked at.
#[derive(Message, Reflect, Debug, Clone, PartialEq, Eq)]
pub struct TerrainImpactOutcome {
    /// Correlates with the announcement that caused it.
    pub batch: TerrainBatchId,
    /// Exactly one entry per announced voxel, in the same canonical order.
    pub voxels: Vec<TerrainVoxelOutcome>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;

    fn at(q: i32, r: i32, level: crate::Level) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    #[test]
    fn a_sorted_deduplicated_volume_is_canonical() {
        let impact = TerrainImpact {
            batch: TerrainBatchId(1),
            volume: vec![at(0, 0, 1), at(0, 0, 2), at(1, 0, 1)],
            element: ElementId(0),
            power: 2,
        };
        assert!(impact.is_canonical());
    }

    /// Order and repeats both break it — the first would make replay depend on
    /// how the volume was built, the second would apply a voxel twice.
    #[test]
    fn unsorted_or_repeated_volumes_are_not_canonical() {
        let unsorted = TerrainImpact {
            batch: TerrainBatchId(1),
            volume: vec![at(1, 0, 1), at(0, 0, 1)],
            element: ElementId(0),
            power: 1,
        };
        assert!(!unsorted.is_canonical());

        let repeated = TerrainImpact {
            volume: vec![at(0, 0, 1), at(0, 0, 1)],
            ..unsorted
        };
        assert!(!repeated.is_canonical());
    }

    #[test]
    fn each_disposition_pins_its_substances() {
        let pos = at(0, 0, 1);
        let stone = Some(SubstanceId(1));
        let dirt = Some(SubstanceId(2));
        let check = |disposition, before, after| {
            TerrainVoxelOutcome {
                pos,
                disposition,
                before,
                after,
            }
            .is_consistent()
        };

        assert!(check(TerrainImpactDisposition::NoMaterial, None, None));
        assert!(check(TerrainImpactDisposition::Resisted, stone, stone));
        assert!(check(TerrainImpactDisposition::Unchanged, stone, stone));
        assert!(check(TerrainImpactDisposition::Cleared, stone, None));
        assert!(check(TerrainImpactDisposition::Replaced, stone, dirt));

        // The ones that would otherwise pass unnoticed.
        assert!(!check(TerrainImpactDisposition::NoMaterial, stone, None));
        assert!(!check(TerrainImpactDisposition::Resisted, stone, dirt));
        assert!(!check(TerrainImpactDisposition::Cleared, stone, stone));
        assert!(
            !check(TerrainImpactDisposition::Replaced, stone, stone),
            "replacing a substance with itself is unchanged, not replaced"
        );
    }
}
