//! Exact authored contract for the V3 Garden recipe.
//!
//! The behavioral lane consumes these coordinates after the recipe vocabulary is
//! published. Keeping the sketch in one typed contract prevents procedural stages
//! from reinterpreting its supports, canopy, lake, source, or entries.

use hex_core::{HexCoord, Level};

/// Garden worlds use exactly this radius.
pub(crate) const REQUIRED_RADIUS: u32 = 12;
/// Runtime level corresponding to elevation zero in the sketch.
pub(crate) const GROUND_LEVEL: Level = 15;
/// Water surface relative to [`GROUND_LEVEL`].
pub(crate) const LAKE_SURFACE_RISE: Level = -1;
/// Gravel-bed surface relative to [`GROUND_LEVEL`].
pub(crate) const LAKE_BED_RISE: Level = -2;
/// Convex architectural enclosure limit used by relief and vegetation.
pub(crate) const COLUMN_ENCLOSURE_LIMIT: i32 = 9;

/// One exact support from the sketch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ColumnContract {
    /// Axial support coordinate.
    pub(crate) coord: HexCoord,
    /// Top surface relative to [`GROUND_LEVEL`].
    pub(crate) relative_top: Level,
}

/// Six regularly spaced supports and their fixed heights.
pub(crate) const COLUMNS: [ColumnContract; 6] = [
    ColumnContract {
        coord: HexCoord::from_axial(-3, -3),
        relative_top: 12,
    },
    ColumnContract {
        coord: HexCoord::from_axial(3, -6),
        relative_top: 12,
    },
    ColumnContract {
        coord: HexCoord::from_axial(-6, 3),
        relative_top: 12,
    },
    ColumnContract {
        coord: HexCoord::from_axial(6, -3),
        relative_top: 10,
    },
    ColumnContract {
        coord: HexCoord::from_axial(3, 3),
        relative_top: 5,
    },
    ColumnContract {
        coord: HexCoord::from_axial(-3, 6),
        relative_top: 8,
    },
];

/// Exact one-voxel-thick canopy joining the three height-12 supports.
pub(crate) const ROOF_CELLS: [HexCoord; 13] = [
    HexCoord::from_axial(-3, -3),
    HexCoord::from_axial(-2, -3),
    HexCoord::from_axial(-1, -4),
    HexCoord::from_axial(0, -4),
    HexCoord::from_axial(1, -5),
    HexCoord::from_axial(2, -5),
    HexCoord::from_axial(3, -6),
    HexCoord::from_axial(-3, -2),
    HexCoord::from_axial(-4, -1),
    HexCoord::from_axial(-4, 0),
    HexCoord::from_axial(-5, 1),
    HexCoord::from_axial(-5, 2),
    HexCoord::from_axial(-6, 3),
];

/// Inclusive axial `r` range for each exact lake `q` row.
pub(crate) const LAKE_ROWS: [(i32, i32, i32); 9] = [
    (-4, 0, 4),
    (-3, -1, 4),
    (-2, -2, 4),
    (-1, -3, 4),
    (0, -4, 4),
    (1, -4, 3),
    (2, -4, 2),
    (3, -4, 1),
    (4, -3, -1),
];

/// The only lake cells horizontally covered by the canopy.
pub(crate) const COVERED_WATER_CELLS: [HexCoord; 2] =
    [HexCoord::from_axial(-4, 0), HexCoord::from_axial(0, -4)];

/// Raised eastern source pool.
pub(crate) const SOURCE_POOL: HexCoord = HexCoord::from_axial(1, 0);
/// Top water voxel of the raised source relative to [`GROUND_LEVEL`].
pub(crate) const SOURCE_POOL_TOP_RISE: Level = 2;
/// Top Stone voxel below the pool and at each shoulder.
pub(crate) const SOURCE_STONE_TOP_RISE: Level = 1;
/// Stone shoulders surrounding the raised source.
pub(crate) const SOURCE_SHOULDERS: [HexCoord; 3] = [
    HexCoord::from_axial(2, -1),
    HexCoord::from_axial(2, 0),
    HexCoord::from_axial(1, 1),
];
/// Continuous vertical fall column.
pub(crate) const SOURCE_FALL: HexCoord = HexCoord::from_axial(1, -1);
/// Top water voxel of the fall relative to [`GROUND_LEVEL`].
pub(crate) const SOURCE_FALL_TOP_RISE: Level = 2;
/// Still-water receiver for the fixed source chain.
pub(crate) const SOURCE_RECEIVER: HexCoord = HexCoord::from_axial(0, -1);
/// Receiver water voxel relative to [`GROUND_LEVEL`].
pub(crate) const SOURCE_RECEIVER_RISE: Level = LAKE_SURFACE_RISE;

/// Boundary entry and shore landing for each protected south-east path.
pub(crate) const PATH_ENDPOINTS: [(HexCoord, HexCoord); 2] = [
    (HexCoord::from_axial(4, 8), HexCoord::from_axial(1, 4)),
    (HexCoord::from_axial(8, 4), HexCoord::from_axial(4, 0)),
];

/// Whether a coordinate belongs to the fixed column-defined enclosure.
pub(crate) fn inside_enclosure(coord: HexCoord) -> bool {
    let q = coord.x();
    let r = coord.y();
    (q + 2 * r).abs() <= COLUMN_ENCLOSURE_LIMIT
        && (q - r).abs() <= COLUMN_ENCLOSURE_LIMIT
        && (2 * q + r).abs() <= COLUMN_ENCLOSURE_LIMIT
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn lake_cells() -> BTreeSet<HexCoord> {
        LAKE_ROWS
            .into_iter()
            .flat_map(|(q, min_r, max_r)| (min_r..=max_r).map(move |r| HexCoord::from_axial(q, r)))
            .collect()
    }

    #[test]
    fn sketch_contract_fixes_all_six_supports_and_heights() {
        assert_eq!(REQUIRED_RADIUS, 12);
        assert_eq!(GROUND_LEVEL, 15);
        assert_eq!(LAKE_SURFACE_RISE, -1);
        assert_eq!(LAKE_BED_RISE, -2);
        assert_eq!(COLUMN_ENCLOSURE_LIMIT, 9);
        assert_eq!(COLUMNS.len(), 6);
        assert_eq!(
            COLUMNS
                .iter()
                .filter(|column| column.relative_top == 12)
                .count(),
            3
        );
        assert_eq!(
            COLUMNS
                .iter()
                .map(|column| column.coord)
                .collect::<BTreeSet<_>>()
                .len(),
            6
        );
        assert!(COLUMNS.iter().all(|column| inside_enclosure(column.coord)));
        assert_eq!(
            COLUMNS
                .iter()
                .map(|column| column.relative_top)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([5, 8, 10, 12])
        );
        assert!(COLUMNS
            .iter()
            .all(|column| column.coord.distance(HexCoord::ORIGIN) == 6));

        let cyclic = [
            HexCoord::from_axial(-3, -3),
            HexCoord::from_axial(3, -6),
            HexCoord::from_axial(6, -3),
            HexCoord::from_axial(3, 3),
            HexCoord::from_axial(-3, 6),
            HexCoord::from_axial(-6, 3),
            HexCoord::from_axial(-3, -3),
        ];
        assert!(cyclic.windows(2).all(|pair| {
            let Some([from, to]) = <&[HexCoord; 2]>::try_from(pair).ok() else {
                unreachable!("a two-cell window always has two entries");
            };
            from.distance(*to) == 6
        }));

        let tall_supports = COLUMNS
            .iter()
            .filter_map(|column| (column.relative_top == 12).then_some(column.coord))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            tall_supports,
            BTreeSet::from([
                HexCoord::from_axial(-3, -3),
                HexCoord::from_axial(3, -6),
                HexCoord::from_axial(-6, 3),
            ])
        );
        let lower_tops = COLUMNS
            .iter()
            .filter_map(|column| (column.relative_top < 12).then_some(column.relative_top))
            .collect::<BTreeSet<_>>();
        assert_eq!(lower_tops, BTreeSet::from([5, 8, 10]));
    }

    #[test]
    fn sketch_contract_fixes_the_connected_thirteen_cell_canopy() {
        let roof = ROOF_CELLS.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(roof.len(), 13);
        assert!(roof.contains(&HexCoord::from_axial(-3, -3)));
        assert!(roof.contains(&HexCoord::from_axial(3, -6)));
        assert!(roof.contains(&HexCoord::from_axial(-6, 3)));
        assert_eq!(
            HexCoord::from_axial(-3, -3).distance(HexCoord::from_axial(-3, -2)),
            1
        );

        for pair in ROOF_CELLS.windows(2) {
            let Some([from, to]) = <&[HexCoord; 2]>::try_from(pair).ok() else {
                unreachable!("a two-cell window always has two entries");
            };
            if *from == HexCoord::from_axial(3, -6) {
                continue;
            }
            assert_eq!(from.distance(*to), 1);
        }
    }

    #[test]
    fn sketch_contract_fixes_the_fifty_nine_cell_lake_and_two_overlaps() {
        let lake = lake_cells();
        let roof = ROOF_CELLS.into_iter().collect::<BTreeSet<_>>();
        let overlap = lake.intersection(&roof).copied().collect::<BTreeSet<_>>();

        assert_eq!(lake.len(), 59);
        assert_eq!(overlap, BTreeSet::from(COVERED_WATER_CELLS));
    }

    #[test]
    fn sketch_contract_fixes_source_and_path_interfaces() {
        let lake = lake_cells();
        assert!(lake.contains(&SOURCE_POOL));
        assert!(lake.contains(&SOURCE_FALL));
        assert!(lake.contains(&SOURCE_RECEIVER));
        assert_eq!(SOURCE_POOL_TOP_RISE, 2);
        assert_eq!(SOURCE_STONE_TOP_RISE, 1);
        assert_eq!(SOURCE_FALL_TOP_RISE, 2);
        assert_eq!(SOURCE_RECEIVER_RISE, -1);
        assert_eq!(SOURCE_FALL_TOP_RISE - SOURCE_RECEIVER_RISE + 1, 4);
        assert!(SOURCE_SHOULDERS
            .into_iter()
            .all(|coord| lake.contains(&coord)));

        let starts = PATH_ENDPOINTS
            .into_iter()
            .map(|(start, _)| start)
            .collect::<BTreeSet<_>>();
        let landings = PATH_ENDPOINTS
            .into_iter()
            .map(|(_, landing)| landing)
            .collect::<BTreeSet<_>>();
        assert_eq!(starts.len(), 2);
        assert_eq!(landings.len(), 2);
        assert!(starts
            .iter()
            .all(|coord| coord.distance(HexCoord::ORIGIN) == 12));
        assert!(landings.iter().all(|coord| {
            !lake.contains(coord)
                && coord
                    .neighbors()
                    .iter()
                    .any(|neighbor| lake.contains(neighbor))
        }));
    }
}
