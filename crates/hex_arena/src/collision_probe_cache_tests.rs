//! Exact candidate memoization limits and lifetime; never cached hit decisions.

use super::*;
use hex_core::arena::ArenaStaticSpan;

fn identities(spans: impl Iterator<Item = Span>) -> Vec<(HexCoord, u32, u32)> {
    spans
        .map(|span| (span.coord, span.bottom.to_bits(), span.top.to_bits()))
        .collect()
}

#[test]
fn probe_cache_bounds_empty_entries_and_does_not_cross_scope_or_clone() {
    let world = CollisionWorld::default();
    let scope = world.probe_scope();
    for x in 0..200 {
        let feet = HexCoord::from_axial(x, 0).to_world(0.0);
        assert!(world.candidates(feet, feet, 0.25).next().is_none());
    }
    let feet = HexCoord::from_axial(199, 0).to_world(0.0);
    assert!(world.candidates(feet, feet, 0.25).next().is_none());
    let stats = scope.stats();
    assert_eq!(
        (stats.misses, stats.hits, stats.peak_entries),
        (200, 1, 128)
    );
    assert!(stats.evictions > 0);
    let cloned = world.clone();
    let clone_scope = cloned.probe_scope();
    assert!(cloned.candidates(feet, feet, 0.25).next().is_none());
    assert_eq!(
        (clone_scope.stats().hits, clone_scope.stats().misses),
        (0, 1)
    );
    drop(scope);
    let next_scope = world.probe_scope();
    assert!(world.candidates(feet, feet, 0.25).next().is_none());
    assert_eq!((next_scope.stats().hits, next_scope.stats().misses), (0, 1));
}

#[test]
fn probe_cache_evicts_spans_and_continues_oversized_candidates_without_truncation() {
    let mut view = ArenaTerrainView::default();
    for x in 0..40 {
        for coord in HexCoord::from_axial(x * 4, 0).within_radius(1) {
            for level in 0..40 {
                view.static_spans.push(ArenaStaticSpan {
                    bottom: TilePos::new(coord, level * 2),
                    top_level: level * 2,
                    blocks_movement: true,
                    blocks_sight: false,
                    blocks_projectiles: false,
                });
            }
        }
    }
    let mut world = CollisionWorld::default();
    world.refresh(&view, ArenaVoxelGeometry::default());
    {
        let scope = world.probe_scope();
        for x in 0..40 {
            let feet = HexCoord::from_axial(x * 4, 0).to_world(0.0);
            assert_eq!(world.candidates(feet, feet, 0.25).count(), 280);
        }
        let stats = scope.stats();
        assert!(stats.evictions > 0);
        assert!(stats.peak_entries <= 128 && stats.peak_spans <= 8192);
    }
    view.revision += 1;
    view.full_rebuild = true;
    view.static_spans = (0..600)
        .map(|level| ArenaStaticSpan {
            bottom: TilePos::new(HexCoord::ORIGIN, level * 2),
            top_level: level * 2,
            blocks_movement: true,
            blocks_sight: false,
            blocks_projectiles: false,
        })
        .collect();
    world.refresh(&view, ArenaVoxelGeometry::default());
    let expected = identities(world.candidates(Vec3::ZERO, Vec3::ZERO, 0.25));
    let scope = world.probe_scope();
    for _ in 0..2 {
        assert_eq!(
            identities(world.candidates(Vec3::ZERO, Vec3::ZERO, 0.25)),
            expected
        );
    }
    assert_eq!(
        (
            scope.stats().oversized,
            scope.stats().hits,
            scope.stats().peak_spans
        ),
        (2, 0, 0)
    );
}

#[test]
fn probe_scope_keeps_long_sweeps_and_nonmovement_masks_uncached() {
    let world = CollisionWorld::default();
    let scope = world.probe_scope();
    for _ in 0..2 {
        for kind in [QueryKind::Sight, QueryKind::Attack] {
            assert!(world
                .candidates_for(Vec3::ZERO, Vec3::ZERO, 0.25, kind)
                .next()
                .is_none());
        }
        assert!(world
            .candidates(Vec3::ZERO, Vec3::X * 40.0, 0.25)
            .next()
            .is_none());
        assert!(world
            .candidates(Vec3::ZERO, Vec3::ZERO, 2.7)
            .next()
            .is_none());
    }
    assert_eq!((scope.stats().hits, scope.stats().misses), (0, 0));
}
