use super::*;
use crate::EncounterTuning;
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, SubstanceId, TilePos};

// Frozen pre-broadphase solver: the order oracle includes every within-pass move,
// not just the initially overlapping pair set or an approximate final distance.
fn brute_force(actors: &mut [Actor], world: &CollisionWorld) -> usize {
    let mut overlaps = 0;
    for _ in 0..4 {
        let mut changed = false;
        for i in 0..actors.len() {
            let (left, right) = actors.split_at_mut(i + 1);
            let Some(a) = left.last_mut() else {
                continue;
            };
            if a.hp <= 0.0 {
                continue;
            }
            for b in right.iter_mut().filter(|a| a.hp > 0.0) {
                if a.species == Species::Worm || b.species == Species::Worm {
                    continue;
                }
                if let Some(push) = body_overlap(a, b) {
                    overlaps += 1;
                    let fa = shapes::slide(world, a, a.feet, push).0;
                    let fb = shapes::slide(world, b, b.feet, -push).0;
                    changed |= fa.distance_squared(a.feet) > SKIN * SKIN
                        || fb.distance_squared(b.feet) > SKIN * SKIN;
                    a.feet = fa;
                    b.feet = fb;
                }
            }
        }
        if !changed {
            break;
        }
    }
    overlaps
}

fn actor(id: u8, feet: Vec3, species: Species) -> Actor {
    let mut actor = Actor::spawn(id, feet, Vec3::NEG_Z);
    actor.configure_species(species, &EncounterTuning::default());
    actor
}

fn exact(actors: &[Actor], world: &CollisionWorld) -> ActorSeparationStats {
    let mut expected = actors.to_vec();
    let expected_overlaps = brute_force(&mut expected, world);
    let mut actual = actors.to_vec();
    let stats = separate_many(&mut actual, world);
    assert_eq!(stats.overlaps, expected_overlaps);
    for (actual, expected) in actual.iter().zip(&expected) {
        assert_eq!(
            actual.feet.to_array().map(f32::to_bits),
            expected.feet.to_array().map(f32::to_bits),
            "actor {} ({:?})",
            actual.id,
            actual.species
        );
        assert_eq!(actual.hp.to_bits(), expected.hp.to_bits());
        assert_eq!(actual.previous_feet, expected.previous_feet);
    }
    stats
}

#[test]
fn separation_requeries_pairs_introduced_by_an_earlier_push_in_the_same_pass() {
    let actors = [
        actor(9, Vec3::X * 1.7, Species::Goblin),
        actor(3, Vec3::X * 1.4, Species::Goblin),
        actor(1, Vec3::X * 2.300_05, Species::Goblin),
    ];
    let mut index = Index::default();
    for (i, actor) in actors.iter().enumerate() {
        index.insert(i, Cells::for_actor(actor));
    }
    let mut initial = Candidates::new(actors.len());
    index.query(0, 0, &mut initial);
    assert_eq!(initial.pop_first(), Some(1));
    assert_eq!(initial.pop_first(), None);
    let stats = exact(&actors, &CollisionWorld::default());
    assert!(stats.queries > actors.len() * stats.passes);
    assert!(stats.overlaps > 1);
}

fn dense_108() -> Vec<Actor> {
    (0_u8..108)
        .map(|id| {
            let mut actor = actor(
                id,
                Vec3::new(
                    f32::from(id % 12) * 0.17 - 1.0,
                    0.0,
                    f32::from(id / 12) * 0.17,
                ),
                Species::Goblin,
            );
            actor.dimensions *= 0.6 + f32::from(id % 7) * 0.23;
            actor
        })
        .collect()
}

#[test]
fn separation_matches_brute_force_for_dense_108_with_heterogeneous_radii() {
    let actors = dense_108();
    let stats = exact(&actors, &CollisionWorld::default());
    assert_eq!(stats.passes, 4);
    assert!(stats.overlaps > 108);
    assert!(stats.candidates <= stats.possible_pairs);
    let mut reversed = actors;
    reversed.reverse();
    exact(&reversed, &CollisionWorld::default());
}

#[test]
fn separation_matches_ordered_solver_for_rotated_dragons_compounds_layers_and_dead_bodies() {
    for seed in 0_u8..8 {
        let actors: Vec<_> = (0_u8..26)
            .map(|id| {
                let species = match id % 8 {
                    0 => Species::Dragon,
                    1 => Species::Golem,
                    2 => Species::Wisp,
                    3 => Species::Worm,
                    4 => Species::Shaman,
                    5 => Species::Human,
                    6 => Species::Shadow,
                    _ => Species::Goblin,
                };
                let x = (u16::from(id) * 17 + u16::from(seed) * 7) % 31;
                let z = (u16::from(id) * 11 + u16::from(seed) * 13) % 29;
                let mut actor = actor(
                    255 - id,
                    Vec3::new(
                        f32::from(x) * 0.18 - 2.0,
                        f32::from(id % 3),
                        f32::from(z) * 0.18 - 2.0,
                    ),
                    species,
                );
                actor.body_yaw = f32::from(id + seed) * 0.37;
                if id % 9 == 0 {
                    actor.hp = 0.0;
                }
                actor
            })
            .collect();
        exact(&actors, &CollisionWorld::default());
    }
}

#[test]
fn separation_preserves_terrain_slide_and_valid_poses_beside_a_wall() {
    let geometry = ArenaVoxelGeometry::default();
    let mut view = ArenaTerrainView::default();
    for coord in HexCoord::ORIGIN.within_radius(8) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    for level in 1..=12 {
        view.voxels
            .insert(TilePos::new(HexCoord::ORIGIN, level), SubstanceId(1));
    }
    let mut world = CollisionWorld::default();
    world.refresh(&view, geometry);
    let mut actors: Vec<_> = (0_u8..20)
        .map(|id| {
            actor(
                id,
                Vec3::new(1.2 + f32::from(id % 4) * 0.1, SKIN, f32::from(id / 4) * 0.1),
                Species::Goblin,
            )
        })
        .collect();
    assert!(actors
        .iter()
        .all(|actor| shapes::clear(&world, actor, actor.feet, actor.body_yaw)));
    exact(&actors, &world);
    separate_many(&mut actors, &world);
    assert!(actors
        .iter()
        .all(|actor| shapes::clear(&world, actor, actor.feet, actor.body_yaw)));
}

fn scattered_108() -> Vec<Actor> {
    (0_u8..108)
        .map(|id| {
            actor(
                id,
                Vec3::new(
                    f32::from(id % 12) * 6.0 - 36.0,
                    0.0,
                    f32::from(id / 12) * 6.0 - 24.0,
                ),
                Species::Goblin,
            )
        })
        .collect()
}

#[test]
fn separation_culls_distant_pairs_and_resets_its_snapshot_with_the_run() {
    let stats = exact(&scattered_108(), &CollisionWorld::default());
    assert_eq!(stats.possible_pairs, 5_778);
    assert_eq!(stats.candidates, 0);
    assert_eq!(stats.overlaps, 0);
    assert_eq!(stats.passes, 1);

    let mut session = crate::ArenaSession::default();
    session.encounter.separation_stats = stats;
    session.reset(
        1,
        &ArenaTerrainView::default(),
        ArenaVoxelGeometry::default(),
    );
    assert_eq!(
        session.actor_separation_stats(),
        ActorSeparationStats::default()
    );
}

#[test]
fn separation_global_fallback_retains_oversized_body_pairs() {
    let mut actors = scattered_108();
    if let Some(actor) = actors.first_mut() {
        actor.dimensions.x = 100.0;
        actor.dimensions.z = 100.0;
    }
    exact(&actors, &CollisionWorld::default());
}

#[test]
#[ignore = "manual CPU sample; no timing assertion"]
fn separation_cpu_sample() {
    for (name, actors) in [("scattered108", scattered_108()), ("dense108", dense_108())] {
        let world = CollisionWorld::default();
        let start = std::time::Instant::now();
        let mut stats = ActorSeparationStats::default();
        for _ in 0..100 {
            let mut trial = actors.clone();
            stats = separate_many(&mut trial, &world);
            std::hint::black_box(trial);
        }
        let broadphase = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let mut trial = actors.clone();
            brute_force(&mut trial, &world);
            std::hint::black_box(trial);
        }
        eprintln!(
            "{name}: broadphase={broadphase:?}, brute_force={:?}, iterations=100, stats={stats:?}",
            start.elapsed()
        );
    }
}
