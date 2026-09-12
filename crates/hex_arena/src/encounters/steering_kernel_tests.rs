//! Synthetic CPU attribution only; actual authored-map profiling remains separate.

use super::*;
use hex_core::arena::{ArenaSolidSpan, ArenaStaticSpan};
use hex_core::SubstanceId;
use std::hint::black_box;
use std::time::{Duration, Instant};

#[test]
#[ignore = "manual bounded CPU attribution; no timing assertion or actual-map claim"]
fn cached_walk_probe_kernel_cpu_attribution() {
    let geometry = ArenaVoxelGeometry {
        radius: 187,
        ..Default::default()
    };
    let mut view = ArenaTerrainView::default();
    view.selection.map = ArenaMap::ForestMassif;
    for coord in HexCoord::ORIGIN.within_radius(geometry.radius) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
        // A distant river keeps the production sorted-liquid broadphase active.
        // Forest poses stay dry; raised canopy exercises non-contact candidates.
        let point = coord.to_world(0.0);
        if point.x.abs() < 12.0 {
            view.liquids.push(ArenaSolidSpan {
                bottom: TilePos::new(coord, 1),
                top_level: 2,
                substance: SubstanceId(5),
            });
        }
        if point.x < -12.0 {
            view.static_spans.push(ArenaStaticSpan {
                bottom: TilePos::new(coord, 20),
                top_level: 24,
                blocks_movement: true,
                blocks_sight: true,
                blocks_projectiles: true,
            });
        }
    }
    view.liquids.sort_unstable_by_key(|run| run.bottom);
    let mut world = CollisionWorld::default();
    world.refresh(&view, geometry);
    let tuning = ArenaTuning::default();
    let actors = (0_u8..100)
        .map(|id| {
            let feet = Vec3::new(
                -75.0 + f32::from(id % 10) * 3.0,
                SKIN,
                -15.0 + f32::from(id / 10) * 3.0,
            );
            let mut actor = Actor::spawn(id, feet, Vec3::X);
            actor.configure_expedition(crate::ExpeditionRole::Goblin, &tuning.encounters);
            actor.grounded = true;
            actor.body.grounded = true;
            actor
        })
        .collect::<Vec<_>>();
    for round in 0..6 {
        let scope = world.probe_scope();
        let mut motion_time = Duration::ZERO;
        let mut bounds_time = Duration::ZERO;
        let mut clearance_time = Duration::ZERO;
        let mut dry_time = Duration::ZERO;
        let mut support_time = Duration::ZERO;
        let mut steps = 0;
        for actor in &actors {
            for angle in [0.0, 0.65, -0.65, 1.3, -1.3] {
                let direction = Quat::from_rotation_y(angle) * Vec3::X;
                let mut body = actor.clone();
                for _ in 0..12 {
                    let start = Instant::now();
                    motion::tick(
                        black_box(&mut body),
                        direction,
                        true,
                        false,
                        false,
                        &world,
                        &tuning.encounters,
                    );
                    motion_time += start.elapsed();
                    let start = Instant::now();
                    let inside = black_box(contained(&body, geometry));
                    bounds_time += start.elapsed();
                    let start = Instant::now();
                    let clear = black_box(shapes::clear(&world, &body, body.feet, body.body_yaw));
                    clearance_time += start.elapsed();
                    let start = Instant::now();
                    let dry = black_box(dry(&body, &view, geometry));
                    dry_time += start.elapsed();
                    let start = Instant::now();
                    let support = black_box(supported(&body, &world));
                    support_time += start.elapsed();
                    assert!(inside && clear && dry && support);
                    steps += 1;
                }
            }
        }
        assert_eq!(steps, 6000);
        assert!(scope.stats().hits > 0);
        if round > 0 {
            println!(
                "PROBE_KERNEL_RECEIPT round={round} steps={steps} motion_ms={} bounds_ms={} clearance_ms={} dry_ms={} support_ms={} cache={:?}",
                motion_time.as_secs_f64() * 1000.0,
                bounds_time.as_secs_f64() * 1000.0,
                clearance_time.as_secs_f64() * 1000.0,
                dry_time.as_secs_f64() * 1000.0,
                support_time.as_secs_f64() * 1000.0,
                scope.stats()
            );
        }
    }
}
