//! Voxel storage: what substance occupies each position in the world.
//!
//! # Columns, bounded by construction
//!
//! A column is a `Vec<SubstanceId>` indexed by level, starting at the bedrock floor.
//! Anything above `len()` is air, so empty sky costs nothing — a world where the
//! tallest peak is level 12 stores twelve entries per column, not twelve thousand.
//!
//! Air *inside* a column is stored explicitly. That is what a cave is: a run of
//! [`SubstanceId::AIR`](hex_core::SubstanceId::AIR) with solid material above and
//! below it.
//!
//! # Why not runs
//!
//! Run-length storage (`Vec<(top, substance)>`) would compress a uniform column to
//! one entry, and was the obvious alternative. Flat voxels won because destruction is
//! the common operation and here it is a single assignment, where a run model has to
//! split an entry, preserve substance on both sides, and merge neighbours back
//! together when they match. At the scale this game works at — radius 20, a few dozen
//! levels — the memory difference is under a megabyte and the correctness difference
//! is what matters.
//!
//! Compression happens at the *rendering* boundary instead: [`crate::grid`] merges
//! vertical runs of the same substance into one prism, so storage stays simple
//! without paying for an entity per voxel.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_core::{Headroom, HexCoord, Level, SubstanceId, TilePos, MAX_HEADROOM};

/// One vertical stack of voxels, from the bedrock floor upward.
///
/// Index is [`Level`]; anything past the end is air.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Column {
    voxels: Vec<SubstanceId>,
}

impl Column {
    /// An empty column — all air, no ground.
    #[must_use]
    pub const fn new() -> Self {
        Self { voxels: Vec::new() }
    }

    /// A column of `substance` from level 0 up to but not including `height`.
    #[must_use]
    pub fn filled(substance: SubstanceId, height: Level) -> Self {
        let count = usize::try_from(height.max(0)).unwrap_or(0);
        Self {
            voxels: vec![substance; count],
        }
    }

    /// The substance at `level`. Out of range, or below the floor, is air.
    #[must_use]
    pub fn get(&self, level: Level) -> SubstanceId {
        usize::try_from(level)
            .ok()
            .and_then(|index| self.voxels.get(index))
            .copied()
            .unwrap_or(SubstanceId::AIR)
    }

    /// Sets the substance at `level`, growing the column with air if needed.
    ///
    /// Filling above the current top is how a bridge or a conjured platform is built:
    /// the gap between is left as air rather than solid.
    ///
    /// Levels below zero are ignored — there is nothing beneath the bedrock floor.
    pub fn set(&mut self, level: Level, substance: SubstanceId) {
        let Ok(index) = usize::try_from(level) else {
            return;
        };
        if index >= self.voxels.len() {
            if substance.is_air() {
                // Clearing air that is already air. Growing the column to store it
                // would be pure waste.
                return;
            }
            self.voxels.resize(index + 1, SubstanceId::AIR);
        }
        if let Some(slot) = self.voxels.get_mut(index) {
            *slot = substance;
        }
        self.trim();
    }

    /// Drops trailing air so `len` means "one past the highest non-air voxel".
    ///
    /// Without this, digging the top off a column would leave the space allocated and
    /// [`Self::top`] would keep reporting the old height — so a piece would try to
    /// stand on a voxel that is no longer there.
    fn trim(&mut self) {
        while self.voxels.last().is_some_and(|s| s.is_air()) {
            self.voxels.pop();
        }
    }

    /// One past the highest non-air level, or 0 for an empty column.
    #[must_use]
    pub fn top(&self) -> Level {
        Level::try_from(self.voxels.len()).unwrap_or(Level::MAX)
    }

    /// The highest non-air level, or [`None`] if the column is entirely air.
    ///
    /// This is the column's visible material surface, which is not necessarily
    /// standable: substances such as water are non-solid. It also says nothing about
    /// what is *below*; a column can have material at the top and air underneath.
    #[must_use]
    pub fn surface(&self) -> Option<Level> {
        self.top()
            .checked_sub(1)
            .filter(|_| !self.voxels.is_empty())
    }

    /// Whether the column has no non-air voxels at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.voxels.is_empty()
    }

    /// The substances from the floor upward.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = SubstanceId> + '_ {
        self.voxels.iter().copied()
    }

    /// Clear voxels starting at `from`, saturated at [`MAX_HEADROOM`].
    ///
    /// A surface passes the exclusive level above its topmost material voxel. A
    /// non-air voxel there means the surface is buried and has zero headroom.
    /// Saturation gives open sky a finite representation even though out-of-range
    /// levels read as air.
    #[must_use]
    pub fn headroom_above(&self, from: Level) -> Headroom {
        let levels = (0..MAX_HEADROOM)
            .take_while(|offset| self.get(from.saturating_add(*offset)).is_air())
            .count()
            .try_into()
            .unwrap_or(MAX_HEADROOM);
        Headroom(levels)
    }
}

/// Axial width and depth of one terrain-storage chunk.
///
/// Euclidean division is important here: coordinates immediately west or north of
/// the origin belong to chunk `-1` with a positive local coordinate, rather than to
/// chunk `0` with a negative local coordinate.
pub(crate) const TERRAIN_CHUNK_SIDE: i32 = 16;
const TERRAIN_CHUNK_SIDE_USIZE: usize = 16;
const TERRAIN_CHUNK_SLOT_COUNT: usize = TERRAIN_CHUNK_SIDE_USIZE * TERRAIN_CHUNK_SIDE_USIZE;

/// Coordinate of a fixed 16 x 16 axial storage chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TerrainChunkCoord {
    pub(crate) q: i32,
    pub(crate) r: i32,
}

/// Coordinate of one column within a terrain chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalColumnCoord {
    q: i32,
    r: i32,
}

impl LocalColumnCoord {
    /// Row-major fixed-slot index: local `r` is the row and local `q` is the column.
    fn slot(self) -> Option<usize> {
        let q = usize::try_from(self.q).ok()?;
        let r = usize::try_from(self.r).ok()?;
        if q >= TERRAIN_CHUNK_SIDE_USIZE || r >= TERRAIN_CHUNK_SIDE_USIZE {
            return None;
        }
        r.checked_mul(TERRAIN_CHUNK_SIDE_USIZE)?
            .checked_add(q)
            .filter(|slot| *slot < TERRAIN_CHUNK_SLOT_COUNT)
    }

    fn from_slot(slot: usize) -> Option<Self> {
        if slot >= TERRAIN_CHUNK_SLOT_COUNT {
            return None;
        }

        Some(Self {
            q: i32::try_from(slot % TERRAIN_CHUNK_SIDE_USIZE).ok()?,
            r: i32::try_from(slot / TERRAIN_CHUNK_SIDE_USIZE).ok()?,
        })
    }
}

/// Splits an axial coordinate into its Euclidean chunk and positive local position.
pub(crate) fn terrain_chunk_coord(coord: HexCoord) -> TerrainChunkCoord {
    split_column_coord(coord).0
}

/// Returns the fixed resident terrain-chunk key for one world axial coordinate.
///
/// The tuple is runtime presentation metadata only. Exact world positions remain
/// the authoritative gameplay and persistence identity. Euclidean division keeps
/// negative coordinates in the same chunks used by [`VoxelMap`].
#[must_use]
pub fn terrain_chunk_key(coord: HexCoord) -> (i32, i32) {
    let chunk = terrain_chunk_coord(coord);
    (chunk.q, chunk.r)
}

fn split_column_coord(coord: HexCoord) -> (TerrainChunkCoord, LocalColumnCoord) {
    (
        TerrainChunkCoord {
            q: coord.x().div_euclid(TERRAIN_CHUNK_SIDE),
            r: coord.y().div_euclid(TERRAIN_CHUNK_SIDE),
        },
        LocalColumnCoord {
            q: coord.x().rem_euclid(TERRAIN_CHUNK_SIDE),
            r: coord.y().rem_euclid(TERRAIN_CHUNK_SIDE),
        },
    )
}

/// Reconstructs an axial coordinate without overflowing at the `i32` boundaries.
fn join_column_coord(chunk: TerrainChunkCoord, local: LocalColumnCoord) -> Option<HexCoord> {
    let q = i64::from(chunk.q)
        .checked_mul(i64::from(TERRAIN_CHUNK_SIDE))?
        .checked_add(i64::from(local.q))?;
    let r = i64::from(chunk.r)
        .checked_mul(i64::from(TERRAIN_CHUNK_SIDE))?
        .checked_add(i64::from(local.r))?;
    Some(HexCoord::from_axial(
        i32::try_from(q).ok()?,
        i32::try_from(r).ok()?,
    ))
}

/// A fixed set of axial column slots. Empty slots cost only their discriminant and
/// let lookup avoid hashing every individual coordinate in a large world.
#[derive(Debug)]
struct TerrainChunk {
    columns: Vec<Option<Column>>,
}

impl Default for TerrainChunk {
    fn default() -> Self {
        Self {
            columns: vec![None; TERRAIN_CHUNK_SLOT_COUNT],
        }
    }
}

impl TerrainChunk {
    fn column(&self, local: LocalColumnCoord) -> Option<&Column> {
        self.columns.get(local.slot()?)?.as_ref()
    }

    /// Returns the column and whether this call populated a previously empty slot.
    fn column_mut_or_insert(&mut self, local: LocalColumnCoord) -> Option<(&mut Column, bool)> {
        let slot = self.columns.get_mut(local.slot()?)?;
        let inserted = slot.is_none();
        Some((slot.get_or_insert_with(Column::new), inserted))
    }

    /// Replaces a slot and reports whether it was previously empty.
    fn insert_column(&mut self, local: LocalColumnCoord, column: Column) -> Option<bool> {
        let slot = self.columns.get_mut(local.slot()?)?;
        let inserted = slot.is_none();
        *slot = Some(column);
        Some(inserted)
    }
}

/// The world, as voxels.
///
/// Private to `hex_map` in every meaningful sense: nothing outside this crate reads
/// it. Terrain reaches the rest of the game as entities carrying
/// [`HexTile`](hex_core::HexTile), [`HexCoord`], [`TilePos`],
/// [`HexSpan`](hex_core::HexSpan), [`SubstanceId`] and
/// [`Headroom`], so storage can be replaced without exposing it
/// outside this crate.
#[derive(Resource, Debug, Default)]
pub struct VoxelMap {
    /// Ordered chunk keys plus row-major fixed slots make iteration deterministic.
    chunks: BTreeMap<TerrainChunkCoord, TerrainChunk>,
    /// Counts generated columns, including deliberately inserted empty columns.
    column_count: usize,
}

impl VoxelMap {
    /// An empty world.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The substance at a position. Anywhere unwritten is air.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> SubstanceId {
        self.column(pos.coord)
            .map_or(SubstanceId::AIR, |column| column.get(pos.level))
    }

    /// Sets the substance at a position, creating the column if needed.
    pub fn set(&mut self, pos: TilePos, substance: SubstanceId) {
        let (chunk_coord, local_coord) = split_column_coord(pos.coord);
        let chunk = self.chunks.entry(chunk_coord).or_default();
        let Some((column, inserted)) = chunk.column_mut_or_insert(local_coord) else {
            return;
        };
        column.set(pos.level, substance);
        if inserted {
            self.column_count = self.column_count.saturating_add(1);
        }
    }

    /// Replaces a whole column.
    pub fn insert_column(&mut self, coord: HexCoord, column: Column) {
        let (chunk_coord, local_coord) = split_column_coord(coord);
        let inserted = self
            .chunks
            .entry(chunk_coord)
            .or_default()
            .insert_column(local_coord, column);
        if inserted == Some(true) {
            self.column_count = self.column_count.saturating_add(1);
        }
    }

    /// The column at a coordinate, if one has been generated.
    #[must_use]
    pub fn column(&self, coord: HexCoord) -> Option<&Column> {
        let (chunk_coord, local_coord) = split_column_coord(coord);
        self.chunks.get(&chunk_coord)?.column(local_coord)
    }

    /// Every generated column in deterministic chunk-major, row-major order.
    pub fn columns(&self) -> impl Iterator<Item = (HexCoord, &Column)> {
        self.chunks.iter().flat_map(|(chunk_coord, chunk)| {
            chunk
                .columns
                .iter()
                .enumerate()
                .filter_map(move |(slot, column)| {
                    let local_coord = LocalColumnCoord::from_slot(slot)?;
                    let coord = join_column_coord(*chunk_coord, local_coord)?;
                    Some((coord, column.as_ref()?))
                })
        })
    }

    /// Every occupied chunk coordinate in deterministic axial order.
    pub(crate) fn chunk_coords(&self) -> impl ExactSizeIterator<Item = TerrainChunkCoord> + '_ {
        self.chunks.keys().copied()
    }

    /// Generated columns in one resident chunk, in deterministic row-major order.
    pub(crate) fn columns_in_chunk(
        &self,
        chunk_coord: TerrainChunkCoord,
    ) -> impl Iterator<Item = (HexCoord, &Column)> {
        self.chunks
            .get(&chunk_coord)
            .into_iter()
            .flat_map(move |chunk| {
                chunk
                    .columns
                    .iter()
                    .enumerate()
                    .filter_map(move |(slot, column)| {
                        let local_coord = LocalColumnCoord::from_slot(slot)?;
                        let coord = join_column_coord(chunk_coord, local_coord)?;
                        Some((coord, column.as_ref()?))
                    })
            })
    }

    /// Number of resident chunks carrying at least one generated column.
    #[must_use]
    pub(crate) fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// How many columns the world holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.column_count
    }

    /// Whether the world is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.column_count == 0
    }

    /// The highest non-air level at a coordinate, if any.
    #[must_use]
    pub fn surface(&self, coord: HexCoord) -> Option<Level> {
        self.column(coord).and_then(Column::surface)
    }
}

/// A contiguous vertical run of one substance within a column.
///
/// This is the unit of *rendering*, not of storage: [`crate::grid`] spawns one entity
/// per run, so a fifteen-level stone column is one prism rather than fifteen. Without
/// it, a radius-20 world with bedrock depth would be tens of thousands of entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubstanceRun {
    /// Lowest level in the run.
    pub bottom: Level,
    /// One past the highest level in the run.
    pub top: Level,
    /// What the whole run is made of.
    pub substance: SubstanceId,
}

impl SubstanceRun {
    /// How many levels the run covers.
    #[must_use]
    pub const fn levels(self) -> Level {
        self.top - self.bottom
    }
}

/// Splits a column into contiguous runs of the same substance, skipping air.
///
/// Air is skipped rather than emitted, which is what makes a cave two runs with a gap
/// between them rather than three runs where the middle one is invisible.
#[must_use]
pub fn runs(column: &Column) -> Vec<SubstanceRun> {
    let mut runs: Vec<SubstanceRun> = Vec::new();

    for (index, substance) in column.iter().enumerate() {
        let level = Level::try_from(index).unwrap_or(Level::MAX);

        if substance.is_air() {
            continue;
        }

        match runs.last_mut() {
            // Extend the run in progress if it is the same substance and directly
            // below this voxel. The second condition is what keeps a cave from
            // welding the rock above it to the rock below.
            Some(run) if run.substance == substance && run.top == level => {
                run.top = level + 1;
            }
            _ => runs.push(SubstanceRun {
                bottom: level,
                top: level + 1,
                substance,
            }),
        }
    }

    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    const STONE: SubstanceId = SubstanceId(1);
    const DIRT: SubstanceId = SubstanceId(2);

    #[test]
    fn an_empty_column_is_all_air() {
        let column = Column::new();
        assert_eq!(column.get(0), SubstanceId::AIR);
        assert_eq!(column.get(50), SubstanceId::AIR);
        assert_eq!(column.top(), 0);
        assert_eq!(column.surface(), None);
    }

    #[test]
    fn filled_columns_report_their_surface() {
        let column = Column::filled(STONE, 5);
        assert_eq!(column.top(), 5);
        assert_eq!(column.surface(), Some(4));
        assert_eq!(column.get(4), STONE);
        assert_eq!(column.get(5), SubstanceId::AIR);
    }

    /// Building above the current top leaves air in between rather than filling it —
    /// this is how a bridge or a floating platform gets made.
    #[test]
    fn setting_above_the_top_leaves_a_gap() {
        let mut column = Column::filled(STONE, 2);
        column.set(6, STONE);

        assert_eq!(column.get(1), STONE);
        assert_eq!(column.get(3), SubstanceId::AIR, "the gap stays empty");
        assert_eq!(column.get(6), STONE);
        assert_eq!(column.top(), 7);
    }

    /// Digging the top voxel has to lower the surface. Leaving trailing air would let
    /// a piece stand on a voxel that is no longer there.
    #[test]
    fn clearing_the_top_lowers_the_surface() {
        let mut column = Column::filled(STONE, 5);
        column.set(4, SubstanceId::AIR);

        assert_eq!(column.top(), 4);
        assert_eq!(column.surface(), Some(3));
    }

    /// Digging a hole in the middle keeps the column's height — the rock above the
    /// hole is still there and still has to be stood on.
    #[test]
    fn clearing_the_middle_keeps_the_height() {
        let mut column = Column::filled(STONE, 6);
        column.set(2, SubstanceId::AIR);

        assert_eq!(column.top(), 6);
        assert_eq!(column.get(2), SubstanceId::AIR);
        assert_eq!(column.get(3), STONE);
    }

    #[test]
    fn levels_below_the_floor_are_ignored() {
        let mut column = Column::filled(STONE, 3);
        column.set(-1, STONE);
        assert_eq!(column.top(), 3, "nothing exists below the bedrock floor");
        assert_eq!(column.get(-1), SubstanceId::AIR);
    }

    #[test]
    fn a_uniform_column_is_one_run() {
        let runs = runs(&Column::filled(STONE, 10));
        let [run] = runs.as_slice() else {
            panic!("uniform stone should merge to a single prism, got {runs:?}")
        };
        assert_eq!(run.bottom, 0);
        assert_eq!(run.top, 10);
        assert_eq!(run.levels(), 10);
    }

    #[test]
    fn alternating_substances_do_not_merge() {
        let mut column = Column::filled(STONE, 4);
        column.set(2, DIRT);
        column.set(3, DIRT);

        let found = runs(&column);
        let [lower, upper] = found.as_slice() else {
            panic!("different substances should not merge, got {found:?}")
        };
        assert_eq!(lower.substance, STONE);
        assert_eq!(upper.substance, DIRT);
        assert_eq!(upper.bottom, 2);
    }

    /// The property the whole cave mechanism rests on: a hole splits one run into
    /// two with a gap, rather than welding the rock above to the rock below.
    #[test]
    fn a_cave_splits_a_column_into_two_runs() {
        let mut column = Column::filled(STONE, 8);
        column.set(3, SubstanceId::AIR);
        column.set(4, SubstanceId::AIR);

        let found = runs(&column);
        let [lower, upper] = found.as_slice() else {
            panic!("the hole should separate the column into two runs, got {found:?}")
        };
        assert_eq!(lower.top, 3, "the lower run stops below the cave");
        assert_eq!(upper.bottom, 5, "the upper run starts above it");
        assert_eq!(upper.top, 8);
    }

    #[test]
    fn an_air_only_column_has_no_runs() {
        let mut column = Column::new();
        column.set(3, STONE);
        column.set(3, SubstanceId::AIR);
        assert!(runs(&column).is_empty());
    }

    /// A floating platform is a run that does not start at the floor.
    #[test]
    fn a_floating_platform_is_one_run_above_the_ground() {
        let mut column = Column::new();
        column.set(8, STONE);
        column.set(9, STONE);

        let found = runs(&column);
        let [platform] = found.as_slice() else {
            panic!("a platform should be one run, got {found:?}")
        };
        assert_eq!(platform.bottom, 8);
        assert_eq!(platform.top, 10);
    }

    #[test]
    fn headroom_handles_buried_surfaces_crawlspaces_and_open_sky() {
        let mut column = Column::filled(STONE, 8);
        column.set(3, SubstanceId::AIR);
        column.set(4, SubstanceId::AIR);

        assert_eq!(column.headroom_above(2), Headroom(0));
        assert_eq!(column.headroom_above(3), Headroom(2));
        assert_eq!(column.headroom_above(4), Headroom(1));
        assert_eq!(column.headroom_above(8), Headroom(MAX_HEADROOM));
    }

    #[test]
    fn the_map_reads_and_writes_by_position() {
        let mut map = VoxelMap::new();
        let pos = TilePos::new(HexCoord::new_cubic(2, -1, -1), 3);

        assert_eq!(map.get(pos), SubstanceId::AIR);
        map.set(pos, STONE);
        assert_eq!(map.get(pos), STONE);
        assert_eq!(map.surface(pos.coord), Some(3));
    }

    /// Stacked positions must not collide — the failure that would make a lower
    /// surface unreachable.
    #[test]
    fn stacked_positions_are_independent() {
        let mut map = VoxelMap::new();
        let coord = HexCoord::ORIGIN;

        map.set(TilePos::new(coord, 0), STONE);
        map.set(TilePos::new(coord, 6), DIRT);

        assert_eq!(map.get(TilePos::new(coord, 0)), STONE);
        assert_eq!(map.get(TilePos::new(coord, 6)), DIRT);
        assert_eq!(map.get(TilePos::new(coord, 3)), SubstanceId::AIR);
    }

    #[test]
    fn chunk_coordinates_use_euclidean_sixteen_by_sixteen_slots() {
        let cases = [
            (
                HexCoord::from_axial(-17, -17),
                TerrainChunkCoord { q: -2, r: -2 },
                LocalColumnCoord { q: 15, r: 15 },
            ),
            (
                HexCoord::from_axial(-16, -16),
                TerrainChunkCoord { q: -1, r: -1 },
                LocalColumnCoord { q: 0, r: 0 },
            ),
            (
                HexCoord::from_axial(-1, -1),
                TerrainChunkCoord { q: -1, r: -1 },
                LocalColumnCoord { q: 15, r: 15 },
            ),
            (
                HexCoord::from_axial(0, 0),
                TerrainChunkCoord { q: 0, r: 0 },
                LocalColumnCoord { q: 0, r: 0 },
            ),
            (
                HexCoord::from_axial(15, 15),
                TerrainChunkCoord { q: 0, r: 0 },
                LocalColumnCoord { q: 15, r: 15 },
            ),
            (
                HexCoord::from_axial(16, 16),
                TerrainChunkCoord { q: 1, r: 1 },
                LocalColumnCoord { q: 0, r: 0 },
            ),
        ];

        for (coord, expected_chunk, expected_local) in cases {
            let (chunk, local) = split_column_coord(coord);
            assert_eq!(chunk, expected_chunk);
            assert_eq!(
                terrain_chunk_key(coord),
                (expected_chunk.q, expected_chunk.r)
            );
            assert_eq!(local, expected_local);
            assert_eq!(join_column_coord(chunk, local), Some(coord));
        }
    }

    #[test]
    fn chunk_split_and_join_round_trip_extreme_coordinates() {
        let values = [
            i32::MIN,
            i32::MIN + 1,
            -33,
            -32,
            -31,
            -17,
            -16,
            -15,
            -1,
            0,
            1,
            15,
            16,
            17,
            31,
            32,
            33,
            i32::MAX - 1,
            i32::MAX,
        ];

        for q in values {
            for r in values {
                let coord = HexCoord::from_axial(q, r);
                let (chunk, local) = split_column_coord(coord);
                assert_eq!(join_column_coord(chunk, local), Some(coord));
                assert!(local.slot().is_some_and(|slot| slot < 256));
            }
        }
    }

    #[test]
    fn map_columns_cross_chunk_boundaries_without_colliding() {
        let mut map = VoxelMap::new();
        let coords = [
            HexCoord::from_axial(-17, 0),
            HexCoord::from_axial(-16, 0),
            HexCoord::from_axial(-1, 0),
            HexCoord::from_axial(0, 0),
            HexCoord::from_axial(15, 0),
            HexCoord::from_axial(16, 0),
        ];

        for (level, coord) in coords.into_iter().enumerate() {
            map.set(
                TilePos::new(coord, Level::try_from(level).unwrap_or(Level::MAX)),
                STONE,
            );
        }

        assert_eq!(map.len(), coords.len());
        for (level, coord) in coords.into_iter().enumerate() {
            let level = Level::try_from(level).unwrap_or(Level::MAX);
            assert_eq!(map.get(TilePos::new(coord, level)), STONE);
            assert_eq!(map.surface(coord), Some(level));
        }
    }

    #[test]
    fn replacing_and_clearing_preserve_generated_column_semantics() {
        let coord = HexCoord::from_axial(-1, 16);
        let mut map = VoxelMap::new();

        // The old flat map retained a generated empty column, including when the
        // first operation merely cleared already-empty air.
        map.set(TilePos::new(coord, 9), SubstanceId::AIR);
        assert_eq!(map.len(), 1);
        assert!(map.column(coord).is_some_and(Column::is_empty));

        map.insert_column(coord, Column::filled(STONE, 4));
        assert_eq!(map.len(), 1, "replacing a slot must not change the count");
        assert_eq!(map.surface(coord), Some(3));

        map.insert_column(coord, Column::new());
        assert_eq!(map.len(), 1);
        assert!(map.column(coord).is_some_and(Column::is_empty));
    }

    #[test]
    fn column_iteration_is_deterministic_across_insertion_orders() {
        let coords = [
            HexCoord::from_axial(31, -17),
            HexCoord::from_axial(-1, 16),
            HexCoord::from_axial(0, 0),
            HexCoord::from_axial(-32, 33),
            HexCoord::from_axial(16, 15),
        ];
        let mut forwards = VoxelMap::new();
        let mut backwards = VoxelMap::new();

        for coord in coords {
            forwards.insert_column(coord, Column::new());
        }
        for coord in coords.into_iter().rev() {
            backwards.insert_column(coord, Column::new());
        }

        let forward_coords: Vec<_> = forwards.columns().map(|(coord, _)| coord).collect();
        let backward_coords: Vec<_> = backwards.columns().map(|(coord, _)| coord).collect();
        assert_eq!(forward_coords, backward_coords);
        assert_eq!(forward_coords.len(), coords.len());
    }

    #[test]
    fn radius_187_uses_the_expected_fixed_chunk_footprint() {
        let mut map = VoxelMap::new();
        for coord in HexCoord::ORIGIN.within_radius(187) {
            map.insert_column(coord, Column::new());
        }

        let occupied_per_chunk = map.chunks.values().map(|chunk| {
            chunk
                .columns
                .iter()
                .filter(|column| column.is_some())
                .count()
        });
        let occupied_total: usize = occupied_per_chunk.clone().sum();
        let fullest_chunk = occupied_per_chunk.max();

        assert_eq!(map.len(), 105_469);
        assert_eq!(map.columns().count(), 105_469);
        assert_eq!(occupied_total, 105_469);
        assert_eq!(map.chunks.len(), 444);
        assert_eq!(fullest_chunk, Some(256));
    }
}
