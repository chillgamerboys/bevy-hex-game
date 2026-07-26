//! The vocabulary for a voxel world: positions, substances, and terrain edits.
//!
//! This module defines *what the map is made of* without saying anything about how
//! it is stored. `hex_map` owns storage, generation, and rendering; everything here
//! is shared so that `hex_gameplay` can reason about terrain without depending on
//! the crate that produces it.
//!
//! # `level`, never `z`
//!
//! Cube coordinates already use `x`, `y` and `z`, and **all three are horizontal** —
//! [`HexCoord::z`](crate::HexCoord::z) returns `-x - y`. The vertical axis is called
//! `level` throughout. It is not called `z` because that name is taken, and not
//! `height` because that reads as "how tall" rather than "how far up".
//!
//! Two different `z`s in one coordinate system produce bugs that are silent and
//! geometric. The name is free; the confusion is not.
//!
//! # Why positions rather than entities
//!
//! A [`TilePos`] identifies one voxel. Entity ids cannot do that job:
//!
//! - They do not survive a world rebuild, so nothing can be saved or restored.
//! - **Interior voxels have no entity at all.** `hex_map` merges vertical runs of the
//!   same substance into a single rendered prism, so the rock two levels inside a
//!   cliff — exactly the thing a tunnelling spell targets — is not an entity.
//!
//! A `TilePos` is stable, serializable, and addresses every voxel whether or not it
//! happens to be visible.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::HexCoord;

/// Vertical index of a voxel. Level 0 is the bedrock floor.
///
/// One level is one movement step, so a traversability rule compares integers rather
/// than floats — no epsilon, no accumulated error. Converting to world units happens
/// once, in `hex_map`, when a run of voxels is turned into a rendered prism.
pub type Level = i32;

/// Identity of a single voxel: which hex, and how far up.
///
/// See the [module documentation](self) for why this rather than an [`Entity`].
#[derive(Component, Reflect, Debug, Default, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[reflect(Component)]
pub struct TilePos {
    /// Which hex column.
    pub coord: HexCoord,
    /// How far up that column. 0 is the bedrock floor.
    pub level: Level,
}

impl TilePos {
    /// The bedrock voxel at the centre of the map.
    pub const ORIGIN: Self = Self {
        coord: HexCoord::ORIGIN,
        level: 0,
    };

    /// A voxel at `coord`, `level` steps up.
    #[must_use]
    pub const fn new(coord: HexCoord, level: Level) -> Self {
        Self { coord, level }
    }

    /// The voxel directly above this one.
    #[must_use]
    pub const fn above(self) -> Self {
        Self {
            coord: self.coord,
            level: self.level + 1,
        }
    }

    /// The voxel directly below this one.
    #[must_use]
    pub const fn below(self) -> Self {
        Self {
            coord: self.coord,
            level: self.level - 1,
        }
    }

    /// The six voxels at the same level in adjacent columns.
    ///
    /// Deliberately excludes [`Self::above`] and [`Self::below`]: **stacked voxels
    /// are not neighbours**. A piece on a bridge cannot step down to the ground
    /// beneath it, so vertical adjacency is not adjacency at all — reaching a lower
    /// column means a ramp of adjacent columns descending gradually, or an ability
    /// that explicitly bypasses the rule.
    #[must_use]
    pub fn neighbours(self) -> [Self; 6] {
        self.coord.neighbors().map(|coord| Self {
            coord,
            level: self.level,
        })
    }

    /// How many levels up `other` is from here. Negative means down.
    ///
    /// This is the quantity a traversability rule compares against a step limit.
    /// With a limit of one level, that is `pos.level_step_to(other).abs() <= 1` — an
    /// integer comparison, which is the point of quantising the vertical axis.
    #[must_use]
    pub const fn level_step_to(self, other: Self) -> Level {
        other.level - self.level
    }

    /// Whether `other` can be reached in one step, ignoring what either voxel is made
    /// of.
    ///
    /// Purely geometric: adjacent column, and no more than `max_step` levels of
    /// climb or drop. Whether the destination is solid enough to stand on, whether
    /// the mover has the movement left, and which abilities ignore this entirely are
    /// all questions for `hex_gameplay`.
    #[must_use]
    pub fn is_within_step_of(self, other: Self, max_step: Level) -> bool {
        self.coord.distance(other.coord) == 1 && self.level_step_to(other).abs() <= max_step
    }
}

/// What a voxel is made of.
///
/// An opaque id rather than an enum, so that adding obsidian is a change to
/// `assets/config/substances.ron` rather than to this crate. The table mapping ids to
/// names and properties is loaded by `hex_assets`.
#[derive(Component, Reflect, Debug, Default, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[reflect(Component)]
pub struct SubstanceId(pub u16);

impl SubstanceId {
    /// Empty space.
    ///
    /// Fixed rather than assigned from the substance table, because "is this voxel
    /// empty" is asked constantly — by movement, by line of sight, by every
    /// destruction effect — and it should be an integer comparison rather than a
    /// table lookup or a string match.
    pub const AIR: Self = Self(0);

    /// Whether this voxel is empty space.
    #[must_use]
    pub const fn is_air(self) -> bool {
        self.0 == Self::AIR.0
    }
}

/// A request to change the world.
///
/// `hex_gameplay` cannot call into `hex_map` — the two crates deliberately cannot see
/// each other — so a spell that digs, builds, or destroys writes one of these and the
/// map applies it. This is the whole write path.
///
/// Reading terrain does not go through here: that is done by querying tile entities
/// for their [`TilePos`], [`HexSpan`](crate::HexSpan) and [`SubstanceId`].
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum TerrainEdit {
    /// Replace the voxel at `pos` with `substance`.
    ///
    /// Filling a position above the current top of a column is legal, and is how a
    /// bridge or a conjured platform gets built. Everything between is left as air.
    Set {
        /// Which voxel to change.
        pos: TilePos,
        /// What it becomes.
        substance: SubstanceId,
    },
    /// Turn the voxel at `pos` into air.
    ///
    /// Digging a voxel out of the middle of a column splits it: the map re-meshes the
    /// column into a run below the hole and a run above it, which is what makes caves
    /// and tunnels fall out of the same mechanism as everything else.
    Clear {
        /// Which voxel to empty.
        pos: TilePos,
    },
}

impl TerrainEdit {
    /// The voxel this edit affects.
    #[must_use]
    pub const fn pos(&self) -> TilePos {
        match *self {
            Self::Set { pos, .. } | Self::Clear { pos } => pos,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_neighbours_are_one_level_apart() {
        let pos = TilePos::new(HexCoord::new_cubic(2, -3, 1), 5);
        assert_eq!(pos.above().level, 6);
        assert_eq!(pos.below().level, 4);
        assert_eq!(pos.above().coord, pos.coord);
    }

    /// The stacking rule, expressed as a property of the type: a bridge and the
    /// ground beneath it share a coordinate but are never neighbours.
    #[test]
    fn stacked_voxels_are_not_neighbours() {
        let ground = TilePos::new(HexCoord::ORIGIN, 0);
        let bridge = TilePos::new(HexCoord::ORIGIN, 6);

        assert!(!ground.neighbours().contains(&bridge));
        assert!(!bridge.neighbours().contains(&ground));
        assert!(!ground.neighbours().contains(&ground.above()));
    }

    #[test]
    fn neighbours_stay_on_the_same_level() {
        let pos = TilePos::new(HexCoord::ORIGIN, 3);
        for neighbour in pos.neighbours() {
            assert_eq!(neighbour.level, 3);
            assert_eq!(pos.coord.distance(neighbour.coord), 1);
        }
    }

    #[test]
    fn level_step_is_signed_and_antisymmetric() {
        let low = TilePos::new(HexCoord::ORIGIN, 2);
        let high = TilePos::new(HexCoord::ORIGIN, 5);

        assert_eq!(low.level_step_to(high), 3);
        assert_eq!(high.level_step_to(low), -3);
    }

    /// With a limit of one level, a gentle ramp is walkable and a cliff is not.
    #[test]
    fn one_level_steps_are_reachable_and_two_are_not() {
        let [east, ..] = TilePos::new(HexCoord::ORIGIN, 4).neighbours();

        let from = TilePos::new(HexCoord::ORIGIN, 4);
        assert!(from.is_within_step_of(east, 1), "level ground is reachable");
        assert!(
            from.is_within_step_of(east.above(), 1),
            "a one-level climb is reachable"
        );
        assert!(
            from.is_within_step_of(east.below(), 1),
            "a one-level drop is reachable"
        );
        assert!(
            !from.is_within_step_of(east.above().above(), 1),
            "a two-level climb is a cliff"
        );
    }

    /// Reachability is horizontal adjacency *and* a small step. A voxel directly
    /// overhead fails the first test even though it passes the second.
    #[test]
    fn a_voxel_directly_above_is_never_reachable() {
        let pos = TilePos::new(HexCoord::ORIGIN, 4);
        assert!(!pos.is_within_step_of(pos.above(), 1));
        assert!(!pos.is_within_step_of(pos.below(), 1));
        assert!(
            !pos.is_within_step_of(pos, 1),
            "nor is standing still a step"
        );
    }

    /// Distant columns are not reachable however similar their heights.
    #[test]
    fn far_columns_are_not_reachable() {
        let here = TilePos::new(HexCoord::ORIGIN, 0);
        let far = TilePos::new(HexCoord::new_cubic(4, -4, 0), 0);
        assert!(!here.is_within_step_of(far, 1));
    }

    #[test]
    fn air_is_recognised_without_a_lookup() {
        assert!(SubstanceId::AIR.is_air());
        assert!(!SubstanceId(1).is_air());
        assert_eq!(SubstanceId::default(), SubstanceId::AIR);
    }

    #[test]
    fn edits_report_the_voxel_they_touch() {
        let pos = TilePos::new(HexCoord::new_cubic(1, 1, -2), 7);
        assert_eq!(TerrainEdit::Clear { pos }.pos(), pos);
        assert_eq!(
            TerrainEdit::Set {
                pos,
                substance: SubstanceId(3)
            }
            .pos(),
            pos
        );
    }

    /// Positions are used as map keys and in save files, so equality and hashing have
    /// to distinguish stacked voxels.
    #[test]
    fn stacked_positions_are_distinct_keys() {
        use std::collections::HashSet;

        let ground = TilePos::new(HexCoord::ORIGIN, 0);
        let bridge = TilePos::new(HexCoord::ORIGIN, 6);

        assert_ne!(ground, bridge);

        let mut set = HashSet::new();
        set.insert(ground);
        set.insert(bridge);
        assert_eq!(set.len(), 2, "a stack must not collapse to one key");
    }
}
