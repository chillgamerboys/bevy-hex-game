//! Exact material occupancy derived from world-published run bounds.
//!
//! The map remains a leaf crate. Gameplay learns which integer voxels contain
//! material only through the inclusive [`RunBottom`](hex_core::RunBottom) to
//! [`TilePos`](hex_core::TilePos) `level` bounds on each
//! [`HexTile`](hex_core::HexTile) entity. This module compacts those bounds by column
//! instead of expanding every voxel into a set.
//!
//! No rendered span, transform, level height, or saturated headroom participates in
//! this projection.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::prelude::*;
use hex_core::{HexCoord, HexTile, Level, RunBottom, TerrainSystems, TilePos};

/// Ordering hook for systems that need the latest exact material occupancy.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainOccupancySystems {
    /// Rebuild [`TerrainOccupancy`] after material-run entities change.
    Publish,
}

/// One malformed material-run publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTerrainRun {
    /// The run's published inclusive top.
    pub top: TilePos,
    /// The run's published inclusive bottom.
    pub bottom: Level,
}

impl fmt::Display for InvalidTerrainRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "material run at {:?} has bottom level {} above its top",
            self.top, self.bottom
        )
    }
}

impl std::error::Error for InvalidTerrainRun {}

/// Exact occupied integer ranges, compacted independently in every hex column.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct TerrainOccupancy {
    columns: BTreeMap<HexCoord, Vec<(Level, Level)>>,
}

/// Faction-authorized material voxels for non-authoritative trajectory consumers.
///
/// Only exact surface positions explicitly present in current faction knowledge enter
/// this projection. It never expands a run, consults [`TerrainOccupancy`], or infers
/// hidden voxels from spans, transforms, level height, or headroom.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KnownTerrainOccupancy {
    voxels: BTreeSet<TilePos>,
}

impl KnownTerrainOccupancy {
    /// Builds authorized occupancy from currently observed material positions.
    #[must_use]
    pub fn from_observed_surfaces(surfaces: impl IntoIterator<Item = TilePos>) -> Self {
        Self {
            voxels: surfaces.into_iter().collect(),
        }
    }

    /// Whether this faction explicitly knows material at `pos` right now.
    #[must_use]
    pub fn contains(&self, pos: TilePos) -> bool {
        self.voxels.contains(&pos)
    }
}

impl TerrainOccupancy {
    /// Builds exact occupancy from inclusive `(top, bottom)` material-run bounds.
    ///
    /// Overlapping or adjacent publications are unioned. This preserves the exact
    /// material fact while remaining indifferent to presentation-only run splits such
    /// as cave cutaway boundaries.
    pub fn from_runs(
        runs: impl IntoIterator<Item = (TilePos, RunBottom)>,
    ) -> Result<Self, InvalidTerrainRun> {
        let mut columns: BTreeMap<HexCoord, Vec<(Level, Level)>> = BTreeMap::new();
        for (top, RunBottom(bottom)) in runs {
            if bottom > top.level {
                return Err(InvalidTerrainRun { top, bottom });
            }
            columns
                .entry(top.coord)
                .or_default()
                .push((bottom, top.level));
        }

        for ranges in columns.values_mut() {
            ranges.sort_unstable();
            let mut compacted: Vec<(Level, Level)> = Vec::with_capacity(ranges.len());
            for (bottom, top) in ranges.drain(..) {
                if let Some((_, previous_top)) = compacted.last_mut() {
                    if bottom <= previous_top.saturating_add(1) {
                        *previous_top = (*previous_top).max(top);
                        continue;
                    }
                }
                compacted.push((bottom, top));
            }
            *ranges = compacted;
        }

        Ok(Self { columns })
    }

    /// Whether this exact integer voxel contains material.
    #[must_use]
    pub fn contains(&self, pos: TilePos) -> bool {
        self.columns.get(&pos.coord).is_some_and(|ranges| {
            ranges
                .binary_search_by(|&(bottom, top)| {
                    if pos.level < bottom {
                        std::cmp::Ordering::Greater
                    } else if pos.level > top {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .is_ok()
        })
    }

    /// Whether no material run has been published.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

/// Registers exact terrain-occupancy publication.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<TerrainOccupancy>().add_systems(
        Update,
        rebuild_terrain_occupancy
            .in_set(TerrainOccupancySystems::Publish)
            .in_set(TerrainSystems::RefreshProjections),
    );
}

/// Rebuilds the compact projection when any material-run entity appears or leaves.
///
/// Terrain edits replace the grid through deferred commands. The changed entities are
/// visible on the following update, when this system rebuilds from the complete new
/// entity set. A malformed run withdraws the projection instead of publishing a
/// plausible-looking partial occupancy.
fn rebuild_terrain_occupancy(
    mut commands: Commands,
    runs: Query<(&TilePos, Option<&RunBottom>), With<HexTile>>,
    added_tiles: Query<(), Added<HexTile>>,
    changed_bottoms: Query<(), (With<HexTile>, Changed<RunBottom>)>,
    mut removed: RemovedComponents<HexTile>,
) {
    let removed_count = removed.read().count();
    if added_tiles.is_empty() && changed_bottoms.is_empty() && removed_count == 0 {
        return;
    }

    let Some(complete) = runs
        .iter()
        .map(|(&top, bottom)| bottom.copied().map(|bottom| (top, bottom)))
        .collect::<Option<Vec<_>>>()
    else {
        error!("withdrawing terrain occupancy: a material-run entity omits RunBottom");
        commands.remove_resource::<TerrainOccupancy>();
        return;
    };

    match TerrainOccupancy::from_runs(complete) {
        Ok(occupancy) => {
            commands.insert_resource(occupancy);
        }
        Err(error) => {
            error!("withdrawing malformed terrain occupancy: {error}");
            commands.remove_resource::<TerrainOccupancy>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_test_app::HeadlessAppBuilder;

    fn occupancy_app() -> HeadlessAppBuilder {
        let mut builder = HeadlessAppBuilder::new();
        plugin(builder.app_mut());
        builder
    }

    #[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestSystems {
        Act,
    }

    #[derive(Resource, Default)]
    struct SeenOccupancy(Vec<bool>);

    fn observe_occupancy(terrain: Res<TerrainOccupancy>, mut seen: ResMut<SeenOccupancy>) {
        seen.0.push(terrain.contains(at(1, 0, 2)));
    }

    fn at(q: i32, r: i32, level: Level) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    #[test]
    fn ordinary_run_includes_exact_bottom_and_top() {
        let occupancy = TerrainOccupancy::from_runs([(at(0, 0, 5), RunBottom(2))])
            .expect("ordinary run should be valid");

        for level in 2..=5 {
            assert!(occupancy.contains(at(0, 0, level)));
        }
        assert!(!occupancy.contains(at(0, 0, 1)));
        assert!(!occupancy.contains(at(0, 0, 6)));
    }

    #[test]
    fn stacked_runs_preserve_the_air_gap_under_a_platform() {
        let occupancy = TerrainOccupancy::from_runs([
            (at(0, 0, 1), RunBottom(0)),
            (at(0, 0, 7), RunBottom(6)),
            (at(1, 0, 4), RunBottom(3)),
        ])
        .expect("stacked runs should be valid");

        assert!(occupancy.contains(at(0, 0, 0)));
        assert!(occupancy.contains(at(0, 0, 1)));
        for level in 2..=5 {
            assert!(
                !occupancy.contains(at(0, 0, level)),
                "level {level} is air below the platform"
            );
        }
        assert!(occupancy.contains(at(0, 0, 6)));
        assert!(occupancy.contains(at(0, 0, 7)));
        assert!(occupancy.contains(at(1, 0, 3)));
        assert!(occupancy.contains(at(1, 0, 4)));
    }

    #[test]
    fn presentation_splits_union_without_filling_real_air() {
        let occupancy = TerrainOccupancy::from_runs([
            (at(0, 0, 2), RunBottom(0)),
            (at(0, 0, 4), RunBottom(3)),
            (at(0, 0, 8), RunBottom(7)),
        ])
        .expect("split runs should be valid");

        for level in 0..=4 {
            assert!(occupancy.contains(at(0, 0, level)));
        }
        assert!(!occupancy.contains(at(0, 0, 5)));
        assert!(!occupancy.contains(at(0, 0, 6)));
        assert!(occupancy.contains(at(0, 0, 7)));
        assert!(occupancy.contains(at(0, 0, 8)));
    }

    #[test]
    fn malformed_bounds_are_rejected_instead_of_inferred() {
        assert_eq!(
            TerrainOccupancy::from_runs([(at(0, 0, 2), RunBottom(3))]),
            Err(InvalidTerrainRun {
                top: at(0, 0, 2),
                bottom: 3,
            })
        );
    }

    #[test]
    fn entity_replacement_republishes_the_complete_stack() {
        let mut app = occupancy_app().build();
        app.world_mut().spawn((HexTile, at(0, 0, 1), RunBottom(0)));
        let platform = app
            .world_mut()
            .spawn((HexTile, at(0, 0, 7), RunBottom(6)))
            .id();
        app.update();

        {
            let occupancy = app.world().resource::<TerrainOccupancy>();
            assert!(occupancy.contains(at(0, 0, 0)));
            assert!(occupancy.contains(at(0, 0, 7)));
            assert!(!occupancy.contains(at(0, 0, 4)));
        }

        app.world_mut().despawn(platform);
        app.world_mut().spawn((HexTile, at(0, 0, 4), RunBottom(3)));
        app.update();

        let occupancy = app.world().resource::<TerrainOccupancy>();
        assert!(occupancy.contains(at(0, 0, 0)));
        assert!(occupancy.contains(at(0, 0, 3)));
        assert!(occupancy.contains(at(0, 0, 4)));
        assert!(!occupancy.contains(at(0, 0, 6)));
        assert!(!occupancy.contains(at(0, 0, 7)));
    }

    #[test]
    fn downstream_actor_sees_same_frame_addition_and_removal() {
        let mut builder = occupancy_app();
        builder.app_mut().init_resource::<SeenOccupancy>();
        builder.app_mut().configure_sets(
            Update,
            TestSystems::Act.after(TerrainOccupancySystems::Publish),
        );
        builder
            .app_mut()
            .add_systems(Update, observe_occupancy.in_set(TestSystems::Act));
        let mut app = builder.build();
        app.world_mut().spawn((HexTile, at(0, 0, 1), RunBottom(0)));
        app.update();

        let added = app
            .world_mut()
            .spawn((HexTile, at(1, 0, 2), RunBottom(2)))
            .id();
        app.update();
        app.world_mut().despawn(added);
        app.update();

        assert_eq!(
            app.world().resource::<SeenOccupancy>().0,
            vec![false, true, false],
            "a consumer ordered after publication must never see prior-frame occupancy"
        );
    }

    #[test]
    fn malformed_entity_publication_withdraws_the_projection() {
        let mut app = occupancy_app().build();
        app.world_mut().spawn((HexTile, at(0, 0, 2), RunBottom(3)));
        app.update();

        assert!(
            !app.world().contains_resource::<TerrainOccupancy>(),
            "partial occupancy must not survive malformed run bounds"
        );
    }

    #[test]
    fn incomplete_entity_publication_withdraws_the_projection() {
        let mut app = occupancy_app().build();
        app.world_mut().spawn((HexTile, at(0, 0, 2)));
        app.update();

        assert!(
            !app.world().contains_resource::<TerrainOccupancy>(),
            "a tile without RunBottom must withdraw the complete projection"
        );
    }

    #[test]
    fn authorized_occupancy_contains_only_explicit_observed_surface_voxels() {
        let known =
            KnownTerrainOccupancy::from_observed_surfaces([at(0, 0, 2), at(0, 0, 7), at(1, 0, 4)]);

        assert!(known.contains(at(0, 0, 2)));
        assert!(known.contains(at(0, 0, 7)));
        assert!(!known.contains(at(0, 0, 1)));
        assert!(!known.contains(at(0, 0, 6)));
    }
}
