//! Preserve the accepted query order while replacing private lookup/tree allocation.

use super::*;
use hex_core::SubstanceId;
use hex_core::arena::{ArenaSolidSpan, ArenaStaticSpan};

fn identities(spans: impl Iterator<Item = Span>) -> Vec<(HexCoord, u32, u32)> {
    spans
        .map(|span| (span.coord, span.bottom.to_bits(), span.top.to_bits()))
        .collect()
}

// Frozen pre-optimization candidate enumeration is the order oracle. It covers
// actual span-vector order too; comparing only a set would hide nearest-hit ties.
fn previous_candidates(
    world: &CollisionWorld,
    start: Vec3,
    end: Vec3,
    radius: f32,
    kind: QueryKind,
) -> Vec<(HexCoord, u32, u32)> {
    let mut ring = 1;
    let mut covered = FACE;
    while radius >= covered {
        ring += 1;
        covered += FACE;
    }
    let coordinates: BTreeSet<_> = HexCoord::from_world(start)
        .line_between(HexCoord::from_world(end))
        .into_iter()
        .flat_map(|coord| coord.within_radius(ring))
        .collect();
    let extra = match kind {
        QueryKind::Movement => &world.static_movement,
        QueryKind::Sight => &world.static_sight,
        QueryKind::Attack => &world.static_attack,
    };
    identities(coordinates.into_iter().flat_map(|coord| {
        [world.columns.get(&coord), extra.get(&coord)]
            .into_iter()
            .flatten()
            .flatten()
            .copied()
    }))
}

#[test]
fn query_cache_preserves_every_ordered_span_across_masks_radii_and_dirty_refresh() {
    let geometry = ArenaVoxelGeometry::default();
    let mut view = ArenaTerrainView {
        revision: 1,
        full_rebuild: true,
        ..Default::default()
    };
    for coord in HexCoord::ORIGIN.within_radius(4) {
        let position = TilePos::new(coord, 0);
        view.voxels.insert(position, SubstanceId(1));
        view.columns.insert(
            coord,
            vec![
                ArenaSolidSpan {
                    bottom: position,
                    top_level: 0,
                    substance: SubstanceId(1),
                },
                ArenaSolidSpan {
                    bottom: TilePos::new(coord, 4),
                    top_level: 5,
                    substance: SubstanceId(1),
                },
            ],
        );
        for (level, movement, sight, attack) in [
            (1, true, false, true),
            (2, false, true, true),
            (3, true, true, false),
        ] {
            view.static_spans.push(ArenaStaticSpan {
                bottom: TilePos::new(coord, level),
                top_level: level,
                blocks_movement: movement,
                blocks_sight: sight,
                blocks_projectiles: attack,
            });
        }
    }
    let mut world = CollisionWorld::default();
    for pass in 0..3 {
        if pass == 1 {
            view.revision += 1;
            view.full_rebuild = false;
            view.columns.remove(&HexCoord::ORIGIN);
            view.voxels.remove(&TilePos::new(HexCoord::ORIGIN, 0));
            view.dirty_columns.insert(HexCoord::ORIGIN);
        }
        if pass == 2 {
            // A consumer which misses a revision must rebuild every column,
            // including one absent from the latest dirty-column announcement.
            view.revision += 2;
            view.columns.insert(
                HexCoord::ORIGIN,
                vec![ArenaSolidSpan {
                    bottom: TilePos::new(HexCoord::ORIGIN, 7),
                    top_level: 9,
                    substance: SubstanceId(1),
                }],
            );
            view.dirty_columns.clear();
        }
        world.refresh(&view, geometry);
        for (start, end) in [
            (Vec3::ZERO, Vec3::ZERO),
            (Vec3::new(0.8, 0.2, 0.45), Vec3::new(0.81, 1.2, 0.46)),
            (Vec3::new(-8.0, 2.0, -4.0), Vec3::new(8.0, -1.0, 4.0)),
            (Vec3::new(8.0, -1.0, 4.0), Vec3::new(-8.0, 2.0, -4.0)),
        ] {
            for radius in [0.0, 0.06, 0.25, FACE, 1.51, 2.7] {
                for kind in [QueryKind::Movement, QueryKind::Sight, QueryKind::Attack] {
                    assert_eq!(
                        identities(world.candidates_for(start, end, radius, kind)),
                        previous_candidates(&world, start, end, radius, kind)
                    );
                }
            }
        }
    }
}
