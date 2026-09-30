//! Reusing immutable candidates cannot change steering or controller trajectories.

use super::*;
use hex_core::arena::{ArenaSolidSpan, ArenaStaticSpan};

#[test]
fn cached_and_uncached_steering_keep_exact_decisions_and_trajectories_through_edits() {
    for species in [
        Species::Human,
        Species::Goblin,
        Species::Shaman,
        Species::Dragon,
    ] {
        let (_, mut view, geometry, _, tuning) = fixture(ArenaEncounter::Goblins);
        view.selection.map = ArenaMap::ForestMassif;
        // Raised bank, isolated trunk and canopy, plus a nearby liquid pocket.
        for coord in HexCoord::ORIGIN.within_radius(geometry.radius) {
            if coord.to_world(0.0).x > 0.0 {
                for level in 1..=3 {
                    view.voxels
                        .insert(TilePos::new(coord, level), SubstanceId(1));
                }
            }
        }
        view.static_spans = [
            ArenaStaticSpan {
                bottom: TilePos::new(HexCoord::from_axial(2, 0), 4),
                top_level: 18,
                blocks_movement: true,
                blocks_sight: true,
                blocks_projectiles: true,
            },
            ArenaStaticSpan {
                bottom: TilePos::new(HexCoord::from_axial(0, 1), 10),
                top_level: 11,
                blocks_movement: true,
                blocks_sight: true,
                blocks_projectiles: true,
            },
        ]
        .into();
        view.liquids = vec![ArenaSolidSpan {
            bottom: TilePos::new(HexCoord::from_axial(1, -2), 4),
            top_level: 6,
            substance: SubstanceId(5),
        }];
        let mut world = CollisionWorld::default();
        world.refresh(&view, geometry);
        let mut baseline = Actor::spawn(7, Vec3::new(-4.0, SKIN, -0.5), Vec3::X);
        baseline.species = species;
        baseline.grounded = true;
        baseline.body.grounded = true;
        if species == Species::Dragon {
            baseline.dimensions = Vec3::new(1.732_050_8, 0.4, 6.0);
        }
        let mut cached = baseline.clone();
        let mut ordinary = steering::Steering::default();
        let mut memoized = steering::Steering::default();
        let mut hits = 0;
        for tick in 1..=720 {
            if tick == 240 || tick == 480 {
                // Full rebuild also changes static masks; no candidate survives
                // the previous tick's immutable scope, even at the same position.
                view.static_spans.clear();
                if tick == 480 {
                    view.voxels
                        .remove(&TilePos::new(HexCoord::from_axial(2, 0), 3));
                }
                view.revision += 1;
                view.full_rebuild = true;
                world.refresh(&view, geometry);
            }
            let desired = if tick < 360 { Vec3::X } else { -Vec3::X };
            let flight = species == Species::Dragon;
            let expected = ordinary.travel(
                &baseline,
                desired,
                flight,
                true,
                &world,
                &view,
                geometry,
                &tuning.encounters,
                tick,
            );
            let scope = world.probe_scope();
            let actual = memoized.travel(
                &cached,
                desired,
                flight,
                true,
                &world,
                &view,
                geometry,
                &tuning.encounters,
                tick,
            );
            hits += scope.stats().hits;
            drop(scope);
            assert_eq!(
                (actual.0.to_array().map(f32::to_bits), actual.1),
                (expected.0.to_array().map(f32::to_bits), expected.1)
            );
            motion::tick(
                &mut baseline,
                expected.0,
                true,
                expected.1,
                flight,
                &world,
                &tuning.encounters,
            );
            motion::tick(
                &mut cached,
                actual.0,
                true,
                actual.1,
                flight,
                &world,
                &tuning.encounters,
            );
            assert_eq!(
                cached.feet.to_array().map(f32::to_bits),
                baseline.feet.to_array().map(f32::to_bits)
            );
            assert_eq!(format!("{:?}", cached.body), format!("{:?}", baseline.body));
            assert_eq!(cached.body_yaw.to_bits(), baseline.body_yaw.to_bits());
            assert_eq!(memoized.blocked_ticks(tick), ordinary.blocked_ticks(tick));
            assert_eq!(memoized.jumping(), ordinary.jumping());
        }
        if species != Species::Dragon {
            assert!(hits > 0);
        }
    }
}
