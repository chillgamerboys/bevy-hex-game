//! Preserve every liquid candidate and real steering trajectory through edits.

use super::*;
use hex_core::arena::ArenaSolidSpan;

fn identities<'a>(
    runs: impl Iterator<Item = &'a ArenaSolidSpan>,
) -> Vec<(TilePos, i32, SubstanceId)> {
    runs.map(|run| (run.bottom, run.top_level, run.substance))
        .collect()
}

#[test]
fn grouped_liquid_candidates_match_ordered_per_column_oracle_after_edits() {
    let mut liquids = HexCoord::ORIGIN
        .within_radius(20)
        .into_iter()
        .filter(|coord| coord.x().rem_euclid(3) != 1)
        .flat_map(|coord| {
            [9, 1, 5, 1].map(|level| ArenaSolidSpan {
                bottom: TilePos::new(coord, level),
                top_level: level + 2,
                substance: SubstanceId(3),
            })
        })
        .collect::<Vec<_>>();
    liquids.sort_by_key(|run| run.bottom);
    let mut candidates_compared = 0;
    for edited in [false, true] {
        if edited {
            liquids.retain(|run| run.bottom.coord.y().rem_euclid(2) == 0);
            liquids.push(ArenaSolidSpan {
                bottom: TilePos::new(HexCoord::ORIGIN, 18),
                top_level: 21,
                substance: SubstanceId(5),
            });
            liquids.sort_by_key(|run| run.bottom);
        }
        for center in [
            HexCoord::ORIGIN,
            HexCoord::from_axial(-8, 3),
            HexCoord::from_axial(19, -7),
            HexCoord::from_axial(60, -40),
        ] {
            for radius in [0, 1, 2, 3, 4, 8, 24] {
                let coords = center.within_radius(radius);
                // Fail explicitly if the upstream neighborhood order changes.
                assert!(coords.windows(2).all(|pair| pair.first() < pair.last()));
                for row in coords.chunk_by(|a, b| a.x() == b.x()) {
                    assert!(row.windows(2).all(|pair| {
                        pair.first()
                            .zip(pair.last())
                            .is_some_and(|(a, b)| b.y() == a.y() + 1)
                    }));
                }
                let original = identities(coords.iter().flat_map(|coord| {
                    let start = liquids.partition_point(|run| run.bottom.coord < *coord);
                    liquids
                        .get(start..)
                        .unwrap_or(&[])
                        .iter()
                        .take_while(move |run| run.bottom.coord == *coord)
                }));
                let grouped = identities(liquid_candidates(&liquids, &coords));
                candidates_compared += original.len();
                assert_eq!(grouped, original);
            }
        }
    }
    assert!(candidates_compared > 10_000);
    assert!(liquid_candidates(&liquids, &[]).next().is_none());
}

#[test]
fn grouped_liquid_steering_matches_full_scan_trajectories_through_pool_edits() {
    for species in [
        Species::Human,
        Species::Goblin,
        Species::Shaman,
        Species::Dragon,
    ] {
        let (_, mut view, geometry, _, tuning) = fixture(ArenaEncounter::Goblins);
        view.selection.map = ArenaMap::ForestMassif;
        view.liquids = [-1, 0, 1]
            .map(|y| ArenaSolidSpan {
                bottom: TilePos::new(HexCoord::from_axial(1, y), 1),
                top_level: 2,
                substance: SubstanceId(5),
            })
            .into();
        view.liquids.sort_by_key(|run| run.bottom);
        let mut world = CollisionWorld::default();
        world.refresh(&view, geometry);
        let mut baseline = Actor::spawn(7, Vec3::new(-4.0, SKIN, -0.5), Vec3::X);
        baseline.configure_species(species, &tuning.encounters);
        baseline.grounded = true;
        baseline.body.grounded = true;
        let mut grouped = baseline.clone();
        let mut ordinary = steering::Steering::default();
        let mut optimized = steering::Steering::default();
        for tick in 1..=360 {
            if tick == 120 || tick == 240 {
                if tick == 120 {
                    view.liquids.push(ArenaSolidSpan {
                        bottom: TilePos::new(HexCoord::from_axial(-1, -1), 1),
                        top_level: 6,
                        substance: SubstanceId(5),
                    });
                    view.liquids.sort_by_key(|run| run.bottom);
                } else {
                    view.liquids.clear();
                }
                view.revision += 1;
                world.refresh(&view, geometry);
            }
            // Only dry() consults this map selector during Steering::travel;
            // Fort preserves its original full liquid scan as the reference.
            let mut reference_view = view.clone();
            reference_view.selection.map = ArenaMap::Fort;
            let desired = if tick < 180 { Vec3::X } else { -Vec3::X };
            let flight = species == Species::Dragon;
            let expected = ordinary.travel(
                &baseline,
                desired,
                flight,
                true,
                &world,
                &reference_view,
                geometry,
                &tuning.encounters,
                tick,
            );
            let actual = optimized.travel(
                &grouped,
                desired,
                flight,
                true,
                &world,
                &view,
                geometry,
                &tuning.encounters,
                tick,
            );
            assert_eq!(
                (actual.0.to_array().map(f32::to_bits), actual.1),
                (expected.0.to_array().map(f32::to_bits), expected.1)
            );
            for (actor, intent) in [(&mut baseline, expected), (&mut grouped, actual)] {
                motion::tick(
                    actor,
                    intent.0,
                    true,
                    intent.1,
                    flight,
                    &world,
                    &tuning.encounters,
                );
            }
            assert_eq!(format!("{grouped:?}"), format!("{baseline:?}"));
            assert_eq!(optimized.blocked_ticks(tick), ordinary.blocked_ticks(tick));
            assert_eq!(optimized.jumping(), ordinary.jumping());
        }
    }
}
