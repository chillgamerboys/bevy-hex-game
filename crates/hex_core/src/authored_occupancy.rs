//! Opt-in exact voxel occupancy published by authored non-terrain objects.
//!
//! Renderer meshes and generic object instances are not gameplay facts. An object
//! that deliberately blocks movement or sight attaches [`AuthoredObjectVoxelRuns`]
//! containing its already-transformed world-grid volume. `hex_units` validates and
//! merges those components into the authoritative runtime projection.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::{Level, TilePos};

/// One inclusive vertical run occupied by an authored object.
///
/// `top.coord` identifies the exact hex column. `bottom..=top.level` is the occupied
/// integer voxel interval in that column. Consumers validate the ordering before
/// publishing authority so reflected or otherwise malformed data cannot become a
/// plausible-looking partial blocker.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthoredObjectVoxelRun {
    /// Inclusive top voxel and exact column.
    pub top: TilePos,
    /// Inclusive lowest occupied level.
    pub bottom: Level,
}

impl AuthoredObjectVoxelRun {
    /// Creates one authored-object voxel run.
    #[must_use]
    pub const fn new(top: TilePos, bottom: Level) -> Self {
        Self { top, bottom }
    }
}

/// Exact occupied volume attached by one opt-in authored object.
///
/// A single object may occupy many columns, so its compact runs travel together on
/// the object root rather than requiring one ECS entity per voxel or per column.
/// Empty lists are valid and contribute no occupancy.
#[derive(Component, Reflect, Debug, Default, Clone, PartialEq, Eq)]
#[reflect(Component)]
pub struct AuthoredObjectVoxelRuns {
    /// Inclusive runs in world-grid coordinates.
    pub runs: Vec<AuthoredObjectVoxelRun>,
}

impl AuthoredObjectVoxelRuns {
    /// Creates one opt-in object publication from exact compact runs.
    #[must_use]
    pub fn new(runs: impl IntoIterator<Item = AuthoredObjectVoxelRun>) -> Self {
        Self {
            runs: runs.into_iter().collect(),
        }
    }

    /// Iterates the authored runs without expanding them to individual voxels.
    pub fn iter(&self) -> impl Iterator<Item = AuthoredObjectVoxelRun> + '_ {
        self.runs.iter().copied()
    }

    /// Whether this object contributes no occupied voxels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::HexCoord;

    use super::*;

    #[test]
    fn one_object_keeps_stacked_column_runs_exact() {
        let coord = HexCoord::from_axial(3, -2);
        let lower = AuthoredObjectVoxelRun::new(TilePos::new(coord, 4), 1);
        let upper = AuthoredObjectVoxelRun::new(TilePos::new(coord, 12), 9);
        let runs = AuthoredObjectVoxelRuns::new([lower, upper]);

        assert_eq!(runs.iter().collect::<Vec<_>>(), vec![lower, upper]);
        assert!(!runs.is_empty());
    }

    #[test]
    fn empty_object_publication_is_explicitly_valid_vocabulary() {
        assert!(AuthoredObjectVoxelRuns::default().is_empty());
    }
}
