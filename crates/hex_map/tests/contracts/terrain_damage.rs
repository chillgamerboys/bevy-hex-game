//! Runtime contracts for map-owned voxel toughness and terrain impacts.

#![expect(
    clippy::expect_used,
    reason = "invalid compile-time fixtures should fail these integration tests immediately"
)]
#![expect(
    clippy::panic,
    reason = "contract fixtures fail immediately when their required shape is absent"
)]

use bevy::ecs::message::MessageCursor;

use super::*;

const TOUGH_SUBSTANCES: [&str; 9] = [
    "grass",
    "snow",
    "dirt",
    "gravel",
    "ice",
    "stone",
    "basalt",
    "worked_stone",
    "metal",
];

fn install_damage_content(app: &mut App) -> (ElementId, ElementId) {
    let source: ElementFile = ron::from_str(
        r#"(
            wheel: ["earth", "fire"],
            fusions: {},
        )"#,
    )
    .expect("the two-element fixture should parse");
    let elements = ElementCatalog::from_file(&source);
    let earth = elements.id("earth").expect("earth should resolve");
    let fire = elements.id("fire").expect("fire should resolve");
    let file = TerrainDamageFile {
        damaging_pairs: TOUGH_SUBSTANCES
            .iter()
            .map(|substance| TerrainDamagePair {
                element: "earth".to_owned(),
                substance: (*substance).to_owned(),
            })
            .collect(),
    };
    let substances = app.world().resource::<SubstanceTable>().clone();
    let damage_table = TerrainDamageTable::from_file(&file, &elements, &substances)
        .expect("the fixture damage matrix should resolve");
    app.insert_resource(elements);
    app.insert_resource(file);
    app.insert_resource(damage_table);
    (earth, fire)
}

fn positions_with_substance(app: &App, name: &str, count: usize) -> Vec<TilePos> {
    let world = app.world();
    let map = world.resource::<VoxelMap>();
    let substance = world
        .resource::<SubstanceTable>()
        .id(name)
        .expect("the requested fixture substance should exist");
    let mut positions = Vec::new();
    for (coord, column) in map.columns() {
        for (index, current) in column.iter().enumerate() {
            if current == substance {
                let level = i32::try_from(index).expect("fixture levels should fit in i32");
                positions.push(TilePos::new(coord, level));
                if positions.len() == count {
                    positions.sort_unstable();
                    return positions;
                }
            }
        }
    }
    panic!("fixture should contain {count} voxel(s) of {name}");
}

fn impact(batch: u64, volume: Vec<TilePos>, element: ElementId, power: u8) -> TerrainImpact {
    TerrainImpact {
        batch: TerrainBatchId(batch),
        volume,
        element,
        power,
    }
}

fn collect_outcomes(
    app: &App,
    cursor: &mut MessageCursor<TerrainImpactOutcome>,
) -> Vec<TerrainImpactOutcome> {
    cursor
        .read(app.world().resource::<Messages<TerrainImpactOutcome>>())
        .cloned()
        .collect()
}

fn exactly_one<T>(values: &[T]) -> &T {
    let [value] = values else {
        panic!("the fixture should contain exactly one value");
    };
    value
}

fn collect_one_outcome(
    app: &App,
    cursor: &mut MessageCursor<TerrainImpactOutcome>,
) -> TerrainImpactOutcome {
    exactly_one(&collect_outcomes(app, cursor)).clone()
}

fn current_grid(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("one terrain grid should exist")
}

fn assert_resisted_without_rebuild(app: &mut App, target: TilePos, element: ElementId, batch: u64) {
    let original = app.world().resource::<VoxelMap>().get(target);
    assert!(
        !original.is_air(),
        "the protected fixture must contain material"
    );
    let grid = current_grid(app);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();
    app.world_mut()
        .write_message(impact(batch, vec![target], element, u8::MAX));
    app.update();
    let outcomes = collect_outcomes(app, &mut cursor);
    let TerrainImpactResult::Applied(voxels) = &exactly_one(&outcomes).result else {
        panic!("a protected valid impact should be applied as resistance");
    };
    assert_eq!(voxels.len(), 1);
    assert_eq!(
        exactly_one(voxels).disposition,
        TerrainImpactDisposition::Resisted
    );
    assert_eq!(app.world().resource::<VoxelMap>().get(target), original);
    assert_eq!(current_grid(app), grid);
    assert_eq!(app.world().resource::<DamagedVoxels>().get(target), None);
}

#[test]
fn partial_then_exact_damage_preserves_then_destroys_the_voxel() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let target = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let stone = app.world().resource::<VoxelMap>().get(target);
    let grid_before = current_grid(&mut app);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();

    let first = impact(1, vec![target], earth, 1);
    app.world_mut().write_message(first.clone());
    app.update();
    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(outcomes.len(), 1);
    let outcome = exactly_one(&outcomes);
    assert!(outcome.is_consistent_with(&first));
    assert_eq!(
        outcome.result,
        TerrainImpactResult::Applied(vec![hex_core::TerrainVoxelOutcome {
            pos: target,
            disposition: TerrainImpactDisposition::Damaged,
            before: Some(stone),
            after: Some(stone),
            health_before: Some(TerrainVoxelHealth {
                remaining: 4,
                maximum: 4,
            }),
            health_after: Some(TerrainVoxelHealth {
                remaining: 3,
                maximum: 4,
            }),
        }])
    );
    assert_eq!(app.world().resource::<VoxelMap>().get(target), stone);
    assert_eq!(
        app.world().resource::<DamagedVoxels>().get(target),
        Some(TerrainVoxelHealth {
            remaining: 3,
            maximum: 4,
        })
    );
    assert_eq!(
        current_grid(&mut app),
        grid_before,
        "partial damage must not rebuild terrain"
    );

    let second = impact(2, vec![target], earth, 3);
    app.world_mut().write_message(second.clone());
    app.update();
    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(outcomes.len(), 1);
    assert!(exactly_one(&outcomes).is_consistent_with(&second));
    assert!(app.world().resource::<VoxelMap>().get(target).is_air());
    assert_eq!(app.world().resource::<DamagedVoxels>().get(target), None);
    assert_ne!(
        current_grid(&mut app),
        grid_before,
        "destruction must use the ordinary terrain rebuild"
    );
}

#[test]
fn impacts_resolve_in_message_and_exact_voxel_order() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let targets = positions_with_substance(&app, "stone", 2);
    let first = impact(10, targets.clone(), earth, 1);
    let second = impact(11, targets.clone(), earth, 1);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();

    app.world_mut().write_message(first.clone());
    app.world_mut().write_message(second.clone());
    app.update();

    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(outcomes.len(), 2);
    let [first_outcome, second_outcome] = outcomes.as_slice() else {
        panic!("the two announced batches should produce exactly two outcomes");
    };
    assert!(first_outcome.is_consistent_with(&first));
    assert!(second_outcome.is_consistent_with(&second));
    for target in targets {
        assert_eq!(
            app.world().resource::<DamagedVoxels>().get(target),
            Some(TerrainVoxelHealth {
                remaining: 2,
                maximum: 4,
            }),
            "the later batch must observe damage from the earlier batch"
        );
    }
}

#[test]
fn damage_tracks_an_exact_internal_voxel_without_exposing_its_neighbours() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let (target, below, above, stone) = {
        let world = app.world();
        let map = world.resource::<VoxelMap>();
        let stone = world
            .resource::<SubstanceTable>()
            .id("stone")
            .expect("stone should exist");
        let (target, below, above) = map
            .columns()
            .find_map(|(coord, column)| {
                (1..column.top().saturating_sub(1)).find_map(|level| {
                    let target = TilePos::new(coord, level);
                    let below = target.below();
                    let above = TilePos::new(coord, level.saturating_add(1));
                    (map.get(below) == stone && map.get(target) == stone && map.get(above) == stone)
                        .then_some((target, below, above))
                })
            })
            .expect("the deep terrain fixture should contain an internal stone voxel");
        (target, below, above, stone)
    };
    let grid_before = current_grid(&mut app);
    let target_has_surface_entity = app
        .world_mut()
        .query_filtered::<&TilePos, With<HexTile>>()
        .iter(app.world())
        .any(|position| *position == target);
    assert!(
        !target_has_surface_entity,
        "an internal voxel must not be reconstructed from a surface entity"
    );

    app.world_mut()
        .write_message(impact(15, vec![target], earth, 1));
    app.update();

    let damaged = app.world().resource::<DamagedVoxels>();
    assert_eq!(
        damaged.get(target),
        Some(TerrainVoxelHealth {
            remaining: 3,
            maximum: 4,
        })
    );
    assert_eq!(damaged.get(below), None);
    assert_eq!(damaged.get(above), None);
    assert_eq!(app.world().resource::<VoxelMap>().get(below), stone);
    assert_eq!(app.world().resource::<VoxelMap>().get(target), stone);
    assert_eq!(app.world().resource::<VoxelMap>().get(above), stone);
    assert_eq!(
        current_grid(&mut app),
        grid_before,
        "partial damage to an internal voxel must not rebuild terrain"
    );
}

#[test]
fn applied_and_rejected_outcomes_keep_incoming_batch_order() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let target = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let valid = impact(13, vec![target], earth, 1);
    let empty = impact(14, Vec::new(), earth, 1);
    let reused = impact(13, vec![target], earth, 1);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();

    app.world_mut().write_message(valid.clone());
    app.world_mut().write_message(empty);
    app.world_mut().write_message(reused);
    app.update();

    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| outcome.batch)
            .collect::<Vec<_>>(),
        vec![TerrainBatchId(13), TerrainBatchId(14), TerrainBatchId(13)]
    );
    let [valid_outcome, empty_outcome, reused_outcome] = outcomes.as_slice() else {
        panic!("the three announced batches should produce exactly three outcomes");
    };
    assert!(matches!(
        valid_outcome.result,
        TerrainImpactResult::Applied(_)
    ));
    assert_eq!(
        empty_outcome.result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::EmptyVolume)
    );
    assert_eq!(
        reused_outcome.result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::ReusedBatch)
    );
    assert!(valid_outcome.is_consistent_with(&valid));
}

#[test]
fn overkill_is_capped_and_empty_voxels_report_no_material() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let stone = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let empty = TilePos::new(
        stone.coord,
        app.world()
            .resource::<VoxelMap>()
            .surface(stone.coord)
            .expect("the target column should have a surface")
            + 5,
    );
    let mut volume = vec![stone, empty];
    volume.sort_unstable();
    let announced = impact(12, volume.clone(), earth, u8::MAX);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();
    app.world_mut().write_message(announced.clone());
    app.update();

    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(outcomes.len(), 1);
    let outcome = exactly_one(&outcomes);
    assert!(outcome.is_consistent_with(&announced));
    let TerrainImpactResult::Applied(voxels) = &outcome.result else {
        panic!("a valid mixed material volume should be applied");
    };
    assert_eq!(
        voxels.iter().map(|voxel| voxel.pos).collect::<Vec<_>>(),
        volume
    );
    assert_eq!(
        voxels
            .iter()
            .find(|voxel| voxel.pos == stone)
            .expect("stone should have an outcome")
            .disposition,
        TerrainImpactDisposition::Destroyed
    );
    assert_eq!(
        voxels
            .iter()
            .find(|voxel| voxel.pos == empty)
            .expect("air should have an outcome")
            .disposition,
        TerrainImpactDisposition::NoMaterial
    );
}

#[test]
fn rejected_batches_are_atomic_and_first_use_is_consumed() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let target = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let original = app.world().resource::<VoxelMap>().get(target);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();

    app.world_mut()
        .write_message(impact(20, Vec::new(), earth, 1));
    app.update();
    assert_eq!(
        collect_one_outcome(&app, &mut cursor).result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::EmptyVolume)
    );

    app.world_mut()
        .write_message(impact(20, vec![target], earth, 4));
    app.update();
    assert_eq!(
        collect_one_outcome(&app, &mut cursor).result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::ReusedBatch)
    );

    let unknown = ElementId(u16::MAX);
    app.world_mut()
        .write_message(impact(21, vec![target], unknown, 1));
    app.update();
    assert_eq!(
        collect_one_outcome(&app, &mut cursor).result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::UnknownElement)
    );

    app.world_mut()
        .write_message(impact(22, vec![target, target], earth, 1));
    app.update();
    assert_eq!(
        collect_one_outcome(&app, &mut cursor).result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::NonCanonicalVolume)
    );

    app.world_mut()
        .write_message(impact(23, vec![target], earth, 0));
    app.update();
    assert_eq!(
        collect_one_outcome(&app, &mut cursor).result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::ZeroPower)
    );

    let _ready = app.world_mut().remove_resource::<TerrainReady>();
    app.world_mut()
        .write_message(impact(24, vec![target], earth, 1));
    app.update();
    assert_eq!(
        collect_one_outcome(&app, &mut cursor).result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::TerrainUnavailable)
    );
    app.insert_resource(TerrainReady);

    app.insert_resource(TerrainDamageFile {
        damaging_pairs: Vec::new(),
    });
    app.world_mut()
        .write_message(impact(25, vec![target], earth, 1));
    app.update();
    assert_eq!(
        collect_one_outcome(&app, &mut cursor).result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::TerrainUnavailable)
    );
    assert_eq!(app.world().resource::<VoxelMap>().get(target), original);
    assert!(app.world().resource::<DamagedVoxels>().is_empty());
}

#[test]
fn missing_matrix_pairs_and_non_diggable_material_resist() {
    let mut app = test_app();
    let (earth, fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let stone = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let bedrock = *exactly_one(&positions_with_substance(&app, "bedrock", 1));
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();

    app.world_mut()
        .write_message(impact(30, vec![stone], fire, 8));
    app.world_mut()
        .write_message(impact(31, vec![bedrock], earth, 8));
    app.update();

    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(outcomes.len(), 2);
    for outcome in outcomes {
        let TerrainImpactResult::Applied(voxels) = outcome.result else {
            panic!("valid resistance fixtures should be applied");
        };
        assert_eq!(voxels.len(), 1);
        assert_eq!(
            exactly_one(&voxels).disposition,
            TerrainImpactDisposition::Resisted
        );
    }
    assert!(app.world().resource::<DamagedVoxels>().is_empty());
}

#[test]
fn direct_replacement_clears_damage_while_same_material_set_does_not_heal() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let target = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let (stone, dirt) = {
        let table = app.world().resource::<SubstanceTable>();
        (
            table.id("stone").expect("stone should exist"),
            table.id("dirt").expect("dirt should exist"),
        )
    };

    app.world_mut()
        .write_message(impact(40, vec![target], earth, 1));
    app.update();
    app.world_mut().write_message(TerrainEdit::Set {
        pos: target,
        substance: stone,
    });
    app.update();
    assert_eq!(
        app.world().resource::<DamagedVoxels>().get(target),
        Some(TerrainVoxelHealth {
            remaining: 3,
            maximum: 4,
        }),
        "a same-material no-op must not become a repair path"
    );

    app.world_mut().write_message(TerrainEdit::Set {
        pos: target,
        substance: dirt,
    });
    app.update();
    assert_eq!(app.world().resource::<DamagedVoxels>().get(target), None);
    app.world_mut()
        .write_message(impact(41, vec![target], earth, 1));
    app.update();
    assert_eq!(
        app.world().resource::<DamagedVoxels>().get(target),
        Some(TerrainVoxelHealth {
            remaining: 1,
            maximum: 2,
        }),
        "the replacement must begin at its own full authored health"
    );

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    assert!(app.world().resource::<VoxelMap>().get(target).is_air());
    assert_eq!(app.world().resource::<DamagedVoxels>().get(target), None);
}

#[test]
fn direct_edits_precede_impacts_and_material_changes_share_one_rebuild() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let target = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let dirt = app
        .world()
        .resource::<SubstanceTable>()
        .id("dirt")
        .expect("dirt should exist");
    let grid_before = current_grid(&mut app);
    let announced = impact(45, vec![target], earth, 2);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();

    app.world_mut().write_message(TerrainEdit::Set {
        pos: target,
        substance: dirt,
    });
    app.world_mut().write_message(announced.clone());
    app.update();

    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(outcomes.len(), 1);
    let outcome = exactly_one(&outcomes);
    assert!(outcome.is_consistent_with(&announced));
    let TerrainImpactResult::Applied(voxels) = &outcome.result else {
        panic!("the edited target should resolve as an applied impact");
    };
    let voxel = exactly_one(voxels);
    assert_eq!(voxel.before, Some(dirt));
    assert_eq!(voxel.disposition, TerrainImpactDisposition::Destroyed);
    assert!(app.world().resource::<VoxelMap>().get(target).is_air());
    assert_ne!(current_grid(&mut app), grid_before);
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<HexGrid>>()
            .iter(app.world())
            .count(),
        1,
        "the combined material changes must publish one final grid"
    );
}

#[test]
fn created_voxels_begin_at_full_health_and_damage_state_clears_on_reentry() {
    let mut app = test_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let coord = HexCoord::ORIGIN;
    let (surface, stone) = {
        let world = app.world();
        (
            world
                .resource::<VoxelMap>()
                .surface(coord)
                .expect("the origin should have a surface"),
            world
                .resource::<SubstanceTable>()
                .id("stone")
                .expect("stone should exist"),
        )
    };
    let created = TilePos::new(coord, surface + 3);
    app.world_mut().write_message(TerrainEdit::Set {
        pos: created,
        substance: stone,
    });
    app.update();
    assert_eq!(app.world().resource::<DamagedVoxels>().get(created), None);

    app.world_mut()
        .write_message(impact(50, vec![created], earth, 1));
    app.update();
    assert_eq!(
        app.world().resource::<DamagedVoxels>().get(created),
        Some(TerrainVoxelHealth {
            remaining: 3,
            maximum: 4,
        })
    );

    // This impact is announced after the final gameplay update and must not leak
    // through OnExit into the fresh session.
    app.world_mut()
        .write_message(impact(51, vec![created], earth, 3));
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(app.world().resource::<DamagedVoxels>().is_empty());
    enter_gameplay(&mut app);
    assert!(app.world().resource::<DamagedVoxels>().is_empty());

    let original_surface = *exactly_one(&positions_with_substance(&app, "stone", 1));
    let announced = impact(51, vec![original_surface], earth, 1);
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();
    app.world_mut().write_message(announced.clone());
    app.update();
    let outcomes = collect_outcomes(&app, &mut cursor);
    assert_eq!(outcomes.len(), 1);
    assert!(exactly_one(&outcomes).is_consistent_with(&announced));
    assert!(
        app.world()
            .resource::<DamagedVoxels>()
            .get(original_surface)
            .is_some(),
        "batch ids are session-local and must reset on re-entry"
    );
}

#[test]
fn authored_liquid_support_protection_resists_impacts() {
    let mut app = v3_waterfall_app();
    let (earth, _fire) = install_damage_content(&mut app);
    enter_gameplay(&mut app);
    let (water_position, support_position, water, support) = {
        let world = app.world();
        let map = world.resource::<VoxelMap>();
        let table = world.resource::<SubstanceTable>();
        let water = table.id("water").expect("water should exist");
        let (water_position, support_position) = map
            .columns()
            .find_map(|(coord, column)| {
                column.iter().enumerate().find_map(|(index, substance)| {
                    let level = i32::try_from(index).ok()?;
                    let water_position = TilePos::new(coord, level);
                    let support_position = water_position.below();
                    (substance == water && table.toughness(map.get(support_position)).is_some())
                        .then_some((water_position, support_position))
                })
            })
            .expect("Waterfall should contain authored water over tough support");
        (
            water_position,
            support_position,
            water,
            map.get(support_position),
        )
    };
    let mut volume = vec![water_position, support_position];
    volume.sort_unstable();
    let mut cursor = app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .get_cursor();
    app.world_mut().write_message(impact(60, volume, earth, 8));
    app.update();

    let outcomes = collect_outcomes(&app, &mut cursor);
    let TerrainImpactResult::Applied(voxels) = &exactly_one(&outcomes).result else {
        panic!("the protected impact should still be applied");
    };
    assert!(voxels
        .iter()
        .all(|voxel| voxel.disposition == TerrainImpactDisposition::Resisted));
    assert_eq!(
        app.world().resource::<VoxelMap>().get(water_position),
        water
    );
    assert_eq!(
        app.world().resource::<VoxelMap>().get(support_position),
        support
    );
    assert!(app.world().resource::<DamagedVoxels>().is_empty());
}

#[test]
fn blocking_feature_roots_and_generated_light_columns_resist_impacts() {
    let mut forest = v3_forest_app();
    let (earth, _fire) = install_damage_content(&mut forest);
    enter_gameplay(&mut forest);
    let tree_root = feature_roots(&mut forest)
        .into_iter()
        .find_map(|(_entity, name, root, _parent)| (name == "GeneratedTree").then_some(root))
        .expect("Forest should publish a protected generated tree root");
    assert_resisted_without_rebuild(&mut forest, tree_root, earth, 70);

    let mut caves = v3_caves_app();
    let (earth, _fire) = install_damage_content(&mut caves);
    enter_gameplay(&mut caves);
    let light_coord = {
        let world = caves.world_mut();
        let mut lights = world.query_filtered::<&TilePos, With<GameplayLight>>();
        lights
            .iter(world)
            .next()
            .copied()
            .expect("Caves should publish a generated gameplay light")
            .coord
    };
    let protected_voxel = {
        let world = caves.world();
        let map = world.resource::<VoxelMap>();
        let table = world.resource::<SubstanceTable>();
        map.column(light_coord)
            .expect("the generated-light column should exist")
            .iter()
            .enumerate()
            .find_map(|(index, substance)| {
                table.toughness(substance).map(|_maximum| {
                    TilePos::new(
                        light_coord,
                        i32::try_from(index).expect("fixture levels should fit in i32"),
                    )
                })
            })
            .expect("the generated-light column should contain tough support")
    };
    assert_resisted_without_rebuild(&mut caves, protected_voxel, earth, 71);
}
