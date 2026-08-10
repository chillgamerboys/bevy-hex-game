//! Authoritative exact occupancy for opt-in authored non-terrain objects.
//!
//! World and object producers attach compact
//! [`AuthoredObjectVoxelRuns`](hex_core::AuthoredObjectVoxelRuns) components. This
//! module validates and unions every complete publication into one gameplay resource
//! before movement and perception run. Generic meshes and object instances never
//! become blockers implicitly.

use std::collections::BTreeMap;
use std::fmt;

use bevy::prelude::*;
use hex_core::{
    AuthoredObjectVoxelRun, AuthoredObjectVoxelRuns, GameplaySetup, HexCoord, Level, Screen,
    TerrainSystems, TilePos, TraversalProfile,
};

use crate::TerrainOccupancySystems;

/// Ordering hook for consumers that need current authored-object occupancy.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthoredObjectOccupancySystems {
    /// Validate and merge every opt-in authored-object run component.
    Publish,
}

/// One malformed authored-object run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidAuthoredObjectRun {
    /// Inclusive published top and exact column.
    pub top: TilePos,
    /// Inclusive published bottom.
    pub bottom: Level,
}

impl fmt::Display for InvalidAuthoredObjectRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "authored-object run at {:?} has bottom level {} above its top",
            self.top, self.bottom
        )
    }
}

impl std::error::Error for InvalidAuthoredObjectRun {}

/// Exact occupied integer ranges, compacted independently in every hex column.
///
/// This resource is always published during a valid gameplay session, including when
/// no authored object opts into obstruction. Its absence is therefore an authority
/// failure rather than another spelling of an empty world.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct AuthoredObjectOccupancy {
    columns: BTreeMap<HexCoord, Vec<(Level, Level)>>,
}

impl AuthoredObjectOccupancy {
    /// Builds exact occupancy from inclusive authored-object runs.
    ///
    /// Overlapping and adjacent inputs are unioned across object roots. Any inverted
    /// run rejects the whole publication so consumers never see a partial volume.
    pub fn from_runs(
        runs: impl IntoIterator<Item = AuthoredObjectVoxelRun>,
    ) -> Result<Self, InvalidAuthoredObjectRun> {
        let mut columns: BTreeMap<HexCoord, Vec<(Level, Level)>> = BTreeMap::new();
        for AuthoredObjectVoxelRun { top, bottom } in runs {
            if bottom > top.level {
                return Err(InvalidAuthoredObjectRun { top, bottom });
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

    /// Whether this exact integer voxel is occupied by an opt-in authored object.
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

    /// Compact inclusive runs in one exact column, ordered from bottom to top.
    #[must_use]
    pub fn column_runs(&self, coord: HexCoord) -> &[(Level, Level)] {
        self.columns.get(&coord).map_or(&[], Vec::as_slice)
    }

    /// Iterates compact inclusive runs in one exact column, bottom first.
    pub fn runs_in_column(&self, coord: HexCoord) -> impl Iterator<Item = (Level, Level)> + '_ {
        self.column_runs(coord).iter().copied()
    }

    /// Whether an authored object overlaps the body standing above `support`.
    ///
    /// The support voxel itself is terrain beneath the body. A profile `N` levels
    /// tall occupies `support.level + 1 ..= support.level + N` in that column.
    /// Non-positive heights and level overflow fail closed.
    #[must_use]
    pub fn blocks_standing_body(&self, support: TilePos, profile: TraversalProfile) -> bool {
        if profile.levels_tall <= 0 {
            return true;
        }
        let Some(bottom) = support.level.checked_add(1) else {
            return true;
        };
        let Some(top) = support.level.checked_add(profile.levels_tall) else {
            return true;
        };
        self.column_runs(support.coord)
            .iter()
            .any(|&(run_bottom, run_top)| run_bottom <= top && run_top >= bottom)
    }

    /// Whether no opt-in object voxel has been published.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Deterministic change key for cached movement projections.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut fingerprint = 14_695_981_039_346_656_037u64;
        for (coord, ranges) in &self.columns {
            for bytes in [coord.x().to_le_bytes(), coord.y().to_le_bytes()] {
                for byte in bytes {
                    fingerprint ^= u64::from(byte);
                    fingerprint = fingerprint.wrapping_mul(1_099_511_628_211);
                }
            }
            for (bottom, top) in ranges {
                for bytes in [bottom.to_le_bytes(), top.to_le_bytes()] {
                    for byte in bytes {
                        fingerprint ^= u64::from(byte);
                        fingerprint = fingerprint.wrapping_mul(1_099_511_628_211);
                    }
                }
            }
        }
        fingerprint
    }
}

/// Registers authored-object occupancy publication and lifecycle.
pub fn plugin(app: &mut App) {
    app.register_type::<AuthoredObjectVoxelRun>()
        .register_type::<AuthoredObjectVoxelRuns>()
        .configure_sets(
            OnEnter(Screen::Gameplay),
            AuthoredObjectOccupancySystems::Publish
                .after(GameplaySetup::Restore)
                .before(GameplaySetup::Perception),
        )
        .configure_sets(
            Update,
            AuthoredObjectOccupancySystems::Publish.after(TerrainOccupancySystems::Publish),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            publish_initial_occupancy.in_set(AuthoredObjectOccupancySystems::Publish),
        )
        .add_systems(
            Update,
            rebuild_occupancy
                .in_set(AuthoredObjectOccupancySystems::Publish)
                .in_set(TerrainSystems::RefreshProjections)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_occupancy);
}

fn publish_initial_occupancy(mut commands: Commands, sources: Query<&AuthoredObjectVoxelRuns>) {
    publish_complete_occupancy(&mut commands, &sources);
}

fn rebuild_occupancy(
    mut commands: Commands,
    sources: Query<&AuthoredObjectVoxelRuns>,
    changed: Query<(), Changed<AuthoredObjectVoxelRuns>>,
    mut removed: RemovedComponents<AuthoredObjectVoxelRuns>,
) {
    if changed.is_empty() && removed.read().next().is_none() {
        return;
    }
    publish_complete_occupancy(&mut commands, &sources);
}

fn publish_complete_occupancy(commands: &mut Commands, sources: &Query<&AuthoredObjectVoxelRuns>) {
    match AuthoredObjectOccupancy::from_runs(sources.iter().flat_map(|source| source.iter())) {
        Ok(occupancy) => {
            commands.insert_resource(occupancy);
        }
        Err(error) => {
            error!("withdrawing malformed authored-object occupancy: {error}");
            commands.remove_resource::<AuthoredObjectOccupancy>();
        }
    }
}

fn clear_occupancy(mut commands: Commands) {
    commands.remove_resource::<AuthoredObjectOccupancy>();
}

#[cfg(test)]
mod tests {
    use hex_test_app::{enter_gameplay, HeadlessAppBuilder};

    use super::*;

    fn at(q: i32, r: i32, level: Level) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    fn run(q: i32, r: i32, bottom: Level, top: Level) -> AuthoredObjectVoxelRun {
        AuthoredObjectVoxelRun::new(at(q, r, top), bottom)
    }

    fn rotate_clockwise(mut q: i32, mut r: i32, rotations: u8) -> (i32, i32) {
        for _ in 0..rotations % 6 {
            (q, r) = (-r, q + r);
        }
        (q, r)
    }

    #[test]
    fn runs_union_without_filling_stacked_air_gaps() {
        let occupancy = AuthoredObjectOccupancy::from_runs([
            run(0, 0, 1, 3),
            run(0, 0, 3, 5),
            run(0, 0, 9, 10),
            run(1, 0, -2, 0),
        ])
        .expect("valid authored runs");

        assert_eq!(occupancy.column_runs(HexCoord::ORIGIN), &[(1, 5), (9, 10)]);
        assert!(occupancy.contains(at(0, 0, 1)));
        assert!(occupancy.contains(at(0, 0, 10)));
        assert!(!occupancy.contains(at(0, 0, 7)));
        assert!(occupancy.contains(at(1, 0, -1)));
    }

    #[test]
    fn malformed_run_rejects_the_complete_projection() {
        assert_eq!(
            AuthoredObjectOccupancy::from_runs([run(0, 0, 4, 3)]),
            Err(InvalidAuthoredObjectRun {
                top: at(0, 0, 3),
                bottom: 4,
            })
        );
    }

    #[test]
    fn standing_overlap_is_exact_and_overflow_fails_closed() {
        let support = at(0, 0, 4);
        let walker = TraversalProfile::WALKER;
        for level in [5, 6] {
            let occupancy = AuthoredObjectOccupancy::from_runs([run(0, 0, level, level)])
                .expect("single voxel");
            assert!(occupancy.blocks_standing_body(support, walker));
        }
        for level in [4, 7] {
            let occupancy = AuthoredObjectOccupancy::from_runs([run(0, 0, level, level)])
                .expect("single voxel");
            assert!(!occupancy.blocks_standing_body(support, walker));
        }
        assert!(
            AuthoredObjectOccupancy::default().blocks_standing_body(at(0, 0, Level::MAX), walker,)
        );
    }

    #[test]
    fn rotated_tapered_runs_keep_exact_columns_and_body_overlap() {
        const BASE: Level = 20;
        const ANCHOR_Q: i32 = 11;
        const ANCHOR_R: i32 = -7;
        let local_runs = [(0, 0, 0, 9), (1, 0, 0, 5), (0, 1, 0, 2), (-1, 1, 0, 0)];

        for rotations in 0..6 {
            let projected = local_runs.into_iter().map(|(q, r, bottom, top)| {
                let (rotated_q, rotated_r) = rotate_clockwise(q, r, rotations);
                run(
                    ANCHOR_Q + rotated_q,
                    ANCHOR_R + rotated_r,
                    BASE + bottom,
                    BASE + top,
                )
            });
            let occupancy =
                AuthoredObjectOccupancy::from_runs(projected).expect("rotated tapered runs");

            for (q, r, bottom, top) in local_runs {
                let (rotated_q, rotated_r) = rotate_clockwise(q, r, rotations);
                let world_q = ANCHOR_Q + rotated_q;
                let world_r = ANCHOR_R + rotated_r;
                assert!(occupancy.contains(at(world_q, world_r, BASE + bottom)));
                assert!(occupancy.contains(at(world_q, world_r, BASE + top)));
                assert!(!occupancy.contains(at(world_q, world_r, BASE + top + 1)));
            }

            let (arm_q, arm_r) = rotate_clockwise(1, 0, rotations);
            let arm_support = at(ANCHOR_Q + arm_q, ANCHOR_R + arm_r, BASE - 1);
            assert!(occupancy.blocks_standing_body(arm_support, TraversalProfile::WALKER));
            assert!(!occupancy.blocks_standing_body(
                at(ANCHOR_Q + arm_q, ANCHOR_R + arm_r, BASE + 5),
                TraversalProfile::WALKER,
            ));
        }
    }

    #[derive(Resource, Default)]
    struct SawInitial(bool);

    fn observe_initial(occupancy: Res<AuthoredObjectOccupancy>, mut observed: ResMut<SawInitial>) {
        observed.0 = occupancy.is_empty();
    }

    #[test]
    fn gameplay_setup_publishes_an_empty_authoritative_resource_before_perception() {
        let mut builder = HeadlessAppBuilder::new().with_states().with_gameplay_sets();
        plugin(builder.app_mut());
        builder.app_mut().init_resource::<SawInitial>().add_systems(
            OnEnter(Screen::Gameplay),
            observe_initial.in_set(GameplaySetup::Perception),
        );
        let mut app = builder.build();

        enter_gameplay(&mut app);

        assert!(app.world().resource::<SawInitial>().0);
        assert!(app.world().contains_resource::<AuthoredObjectOccupancy>());
    }

    #[test]
    fn source_changes_republish_and_malformed_data_withdraws_authority() {
        let mut builder = HeadlessAppBuilder::new().with_states().with_gameplay_sets();
        plugin(builder.app_mut());
        let mut app = builder.build();
        let source = app
            .world_mut()
            .spawn(AuthoredObjectVoxelRuns::new([run(1, 0, 2, 4)]))
            .id();
        enter_gameplay(&mut app);
        assert!(app
            .world()
            .resource::<AuthoredObjectOccupancy>()
            .contains(at(1, 0, 3)));

        app.world_mut()
            .entity_mut(source)
            .insert(AuthoredObjectVoxelRuns::new([run(2, 0, 7, 9)]));
        app.update();
        let occupancy = app.world().resource::<AuthoredObjectOccupancy>();
        assert!(!occupancy.contains(at(1, 0, 3)));
        assert!(occupancy.contains(at(2, 0, 8)));

        app.world_mut()
            .entity_mut(source)
            .insert(AuthoredObjectVoxelRuns::new([run(0, 0, 5, 4)]));
        app.update();
        assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());
    }

    #[test]
    fn removing_last_source_publishes_valid_empty_occupancy_and_exit_clears_it() {
        let mut builder = HeadlessAppBuilder::new().with_states().with_gameplay_sets();
        plugin(builder.app_mut());
        let mut app = builder.build();
        let source = app
            .world_mut()
            .spawn(AuthoredObjectVoxelRuns::new([run(0, 0, 1, 2)]))
            .id();
        enter_gameplay(&mut app);

        app.world_mut().despawn(source);
        app.update();
        assert!(app.world().resource::<AuthoredObjectOccupancy>().is_empty());

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());
    }

    #[test]
    fn gameplay_reentry_rebuilds_persisted_authored_sources() {
        let mut builder = HeadlessAppBuilder::new().with_states().with_gameplay_sets();
        plugin(builder.app_mut());
        let mut app = builder.build();
        app.world_mut()
            .spawn(AuthoredObjectVoxelRuns::new([run(3, -2, 8, 14)]));

        enter_gameplay(&mut app);
        assert!(app
            .world()
            .resource::<AuthoredObjectOccupancy>()
            .contains(at(3, -2, 11)));

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());

        enter_gameplay(&mut app);
        assert!(app
            .world()
            .resource::<AuthoredObjectOccupancy>()
            .contains(at(3, -2, 11)));
    }
}
